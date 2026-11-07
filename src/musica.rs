//! El visualizador: cava saca las bandas del audio y aquí se vuelven barras en
//! el teclado. Son cuadros en vivo: no se guarda nada en el teclado, y unos 2 s
//! después del último cuadro regresa solo a su luz. Sin música no se manda nada.

use crate::hid::Teclado;
use crate::luz::{self, Rgb};
use crate::{l, modo_actual, poner_modo};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

/// Con 11 bandas un cuadro con todo prendido cabe en 9 paquetes: más largos se pierden.
pub const BANDAS: usize = 11;
const COLUMNAS: usize = 15;
const FILAS: usize = 6;
const MAX_BYTES: usize = 126;
/// Más rápido se pierden cuadros por radio, y por cable pudo colgar al teclado.
const ENTRE_CUADROS: Duration = Duration::from_millis(66);
const SILENCIO: Duration = Duration::from_secs(2);
const REVISAR_ENLACE: Duration = Duration::from_secs(30);
const REVISAR_COLOR: Duration = Duration::from_secs(5);
const REINTENTO: Duration = Duration::from_secs(3);
/// Abajo de esto no es música: es lo que queda del suavizado de cava.
const PISO: u8 = 3;

pub type Grupo = (Rgb, Vec<u8>);

/// Led → banda, de izquierda (graves) a derecha (agudos).
fn banda(led: usize) -> usize {
    (led / FILAS).min(COLUMNAS - 1) * BANDAS / COLUMNAS
}

/// Cuántas teclas prende este nivel, de abajo para arriba.
fn alto(nivel: u8) -> usize {
    if nivel < PISO { 0 } else { (nivel as usize * FILAS).div_ceil(255) }
}

/// Una banda es un grupo con su color, y más fuerte también brilla más.
pub fn grupos(niveles: &[u8], colores: &[Rgb; BANDAS]) -> Vec<Grupo> {
    let mut out = Vec::new();
    for (i, nivel) in niveles.iter().take(BANDAS).enumerate() {
        let a = alto(*nivel);
        let leds: Vec<u8> = luz::TECLAS
            .iter()
            .map(|k| k.1)
            .filter(|led| banda(*led) == i && led % FILAS >= FILAS - a)
            .map(|led| led as u8)
            .collect();
        if !leds.is_empty() {
            let f = 0.45 + 0.55 * *nivel as f32 / 255.0;
            let (r, g, b) = colores[i];
            let m = |x: u8| (x as f32 * f) as u8;
            out.push(((m(r), m(g), m(b)), leds));
        }
    }
    out
}

/// Por radio: R G B cuántas led led…, uno tras otro. Las que no van quedan apagadas.
pub fn a_bytes(gr: &[Grupo]) -> Vec<u8> {
    if gr.is_empty() {
        return vec![0, 0, 0, 1, 0];
    }
    let mut b = Vec::with_capacity(MAX_BYTES);
    for ((r, g, a), leds) in gr {
        b.extend_from_slice(&[*r, *g, *a, leds.len() as u8]);
        b.extend_from_slice(leds);
    }
    b
}

/// Base en el color del wallpaper, y el tono se va corriendo de graves a agudos.
fn colores_de(base: Rgb) -> [Rgb; BANDAS] {
    let (h, s, _) = luz::rgb_a_hsv(base);
    std::array::from_fn(|i| luz::hsv_a_rgb(h + (i as f32 / (BANDAS - 1) as f32 - 0.5) * 80.0, s, 1.0))
}

fn wallpaper_de_ahora() -> Option<PathBuf> {
    let link = PathBuf::from(std::env::var("HOME").ok()?).join(".config/hypr/wallpaper_effects/.wallpaper_link");
    std::fs::canonicalize(link).ok()
}

fn runtime() -> PathBuf {
    PathBuf::from(std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into()))
}

const CAVA: &str = "[general]
bars = 11
framerate = 20
[input]
method = pulse
source = auto
[output]
method = raw
channels = mono
raw_target = /dev/stdout
data_format = binary
bit_format = 8bit
";

/// Lanza cava y deja el último cuadro en un canal. El hilo se acaba cuando cava muere.
fn lanzar_cava() -> Result<(Child, Receiver<[u8; BANDAS]>), String> {
    let conf = runtime().join("chispa-cava.conf");
    std::fs::write(&conf, CAVA).map_err(|e| format!("{}: {e}", conf.display()))?;
    let mut hijo = Command::new("cava")
        .arg("-p")
        .arg(&conf)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cava: {e}"))?;
    let mut salida = hijo.stdout.take().ok_or("cava: sin salida")?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut b = [0u8; BANDAS];
        while salida.read_exact(&mut b).is_ok() && tx.send(b).is_ok() {}
    });
    Ok((hijo, rx))
}

/// Uno a la vez. Regresa None si ya hay otro corriendo.
fn solo_uno() -> Result<Option<File>, String> {
    let ruta = runtime().join("chispa-music.lock");
    let f = OpenOptions::new().create(true).truncate(false).write(true).open(&ruta).map_err(|e| format!("{}: {e}", ruta.display()))?;
    // SAFETY: f sigue abierto; flock solo recibe su descriptor.
    Ok((unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0).then_some(f))
}

static CTRL_C: AtomicBool = AtomicBool::new(false);

extern "C" fn al_ctrl_c(_: libc::c_int) {
    CTRL_C.store(true, Ordering::SeqCst);
}

pub fn correr(args: &[String]) -> Result<(), String> {
    let opcion = args.first().map(String::as_str);
    match opcion {
        None | Some("--auto") => {}
        Some("stop") => {
            if modo_actual().as_deref() == Some("music") {
                poner_modo("custom")?;
            }
            return Ok(());
        }
        Some(o) => return Err(format!("{}: {o} (chispa help)", l("no conozco esa opción", "unknown option"))),
    }
    let auto = opcion.is_some();
    if auto && modo_actual().as_deref() != Some("music") {
        return Ok(());
    }
    let Some(_uno) = solo_uno()? else {
        poner_modo("music")?;
        if !auto {
            println!("{}", l("ya está corriendo", "already running"));
        }
        return Ok(());
    };
    // a mano: si no hay teclado se dice de una vez; al entrar a la sesión se espera
    let mut t = match Teclado::abrir() {
        Ok(t) => Some(t),
        Err(_) if auto => None,
        Err(e) => return Err(e),
    };
    if let Some(t) = &t {
        t.soltar();
    }
    let (mut hijo, rx) = lanzar_cava()?;
    poner_modo("music")?;
    // SAFETY: al_ctrl_c solo toca un atómico.
    unsafe { libc::signal(libc::SIGINT, al_ctrl_c as *const () as libc::sighandler_t) };
    if !auto {
        println!("{}", l("visualizador prendido; Ctrl+C para parar", "visualizer on; Ctrl+C to stop"));
    }
    let r = bucle(&mut t, &rx);
    let _ = hijo.kill();
    let _ = hijo.wait();
    if CTRL_C.load(Ordering::SeqCst) {
        return poner_modo("custom");
    }
    r
}

fn bucle(t: &mut Option<Teclado>, rx: &Receiver<[u8; BANDAS]>) -> Result<(), String> {
    let mut wall = wallpaper_de_ahora();
    let color = |w: &Option<PathBuf>| w.as_ref().and_then(|p| crate::color_del_wallpaper(p.to_str()).ok()).unwrap_or((0, 200, 255));
    let mut colores = colores_de(color(&wall));
    let mut ultima_musica: Option<Instant> = None;
    let mut ultimo_cuadro = Instant::now() - ENTRE_CUADROS;
    let mut enlace_visto = Instant::now();
    let mut color_visto = Instant::now();
    let mut modo_visto = Instant::now();
    loop {
        if CTRL_C.load(Ordering::SeqCst) {
            return Ok(());
        }
        // lo que se escoge a mano (o desde el menú) apaga el visualizador
        if modo_visto.elapsed() >= Duration::from_secs(1) {
            modo_visto = Instant::now();
            if modo_actual().as_deref() != Some("music") {
                return Ok(());
            }
        }
        let niveles = match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(n) => rx.try_iter().last().unwrap_or(n),
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                if CTRL_C.load(Ordering::SeqCst) {
                    return Ok(());
                }
                return Err(l("cava se murió", "cava died"));
            }
        };
        if niveles.iter().any(|n| *n >= PISO) {
            ultima_musica = Some(Instant::now());
        }
        if ultima_musica.is_none_or(|u| u.elapsed() > SILENCIO) {
            continue;
        }
        if ultimo_cuadro.elapsed() < ENTRE_CUADROS {
            continue;
        }
        if color_visto.elapsed() >= REVISAR_COLOR {
            color_visto = Instant::now();
            let w = wallpaper_de_ahora();
            if w != wall {
                wall = w;
                colores = colores_de(color(&wall));
            }
        }
        // el cable pudo llegar o irse: se vuelve a buscar de vez en cuando
        if t.is_none() || enlace_visto.elapsed() >= REVISAR_ENLACE {
            enlace_visto = Instant::now();
            if let Ok(n) = Teclado::abrir() {
                n.soltar();
                *t = Some(n);
            }
        }
        let Some(tec) = t.as_ref() else {
            enlace_visto = Instant::now() - REVISAR_ENLACE + REINTENTO;
            continue;
        };
        ultimo_cuadro = Instant::now();
        if tec.cuadro(&grupos(&niveles, &colores)).is_err() {
            // se fue (lo desconectaron, cambió de modo): se sigue buscando
            *t = None;
            enlace_visto = Instant::now() - REVISAR_ENLACE + REINTENTO;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const C: [Rgb; BANDAS] = [(255, 0, 0); BANDAS];

    #[test]
    fn todo_prendido_cabe_en_un_cuadro() {
        let b = a_bytes(&grupos(&[255; BANDAS], &C));
        assert!(b.len() <= MAX_BYTES, "{} bytes", b.len());
    }

    #[test]
    fn cada_tecla_cae_en_una_banda_valida_y_todas_se_usan() {
        let mut usadas = [false; BANDAS];
        for k in luz::TECLAS {
            usadas[banda(k.1)] = true;
        }
        assert!(usadas.iter().all(|u| *u));
    }

    #[test]
    fn las_barras_crecen_de_abajo_para_arriba() {
        let mut n = [0u8; BANDAS];
        n[0] = 43; // una fila: la de abajo, lctrl
        let g = grupos(&n, &C);
        assert_eq!(g.len(), 1);
        assert!(g[0].1.contains(&5));
        assert!(!g[0].1.contains(&0), "esc está hasta arriba");
        n[0] = 255;
        let g = grupos(&n, &C);
        assert!(g[0].1.contains(&0) && g[0].1.contains(&5));
    }

    #[test]
    fn los_graves_a_la_izquierda_los_agudos_a_la_derecha() {
        assert_eq!(banda(5), 0, "lctrl");
        assert_eq!(banda(89), BANDAS - 1, "flecha derecha");
    }

    #[test]
    fn sin_nivel_no_prende_nada() {
        assert!(grupos(&[0; BANDAS], &C).is_empty());
        assert_eq!(a_bytes(&[]), vec![0, 0, 0, 1, 0], "un cuadro apagado sigue siendo válido");
    }

    #[test]
    fn el_tono_se_corre_entre_bandas() {
        let c = colores_de((255, 0, 0));
        assert_ne!(c[0], c[BANDAS - 1]);
    }
}
