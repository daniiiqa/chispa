//! `chispa video`: con un wallpaper en video (mpvpaper), el teclado es una
//! pantallita del wallpaper: cada tecla toma el color del pedazo de video que le
//! queda encima. ffmpeg decodifica el mismo video en la GPU (VAAPI: un 4K por
//! CPU costaba más de medio núcleo, por VAAPI casi nada), achicado a 48×18, y
//! se sincroniza con mpvpaper preguntándole por su socket dónde va.
//!
//! Solo por radio: un cuadro aguanta 126 bytes, así que los colores se juntan
//! en 10 (10 × 4 + 80 teclas = 120). Por cable se pausa: ahí los cuadros en
//! vivo hacen que el teclado se coma las teclas. Nada se
//! guarda: al irse, el teclado regresa a lo guardado (o a lo que sostiene
//! `chispa hold`, que se hace a un lado mientras corre esto).

use crate::hid::{Enlace, Teclado};
use crate::luz::{self, Rgb};
use crate::{l, sigue_wallpaper};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::thread::sleep;
use std::time::{Duration, Instant};

const ANCHO: usize = 48;
const ALTO: usize = 18;
const CUADRO: usize = ANCHO * ALTO * 3;
/// por radio un cuadro de 10 colores son 9 paquetes, cada uno ~7 ms (5 de
/// aire, más pegados se pierden): medido, 63 ms por cuadro. El techo es ~15;
/// con menos colores no baja, las 80 teclas ya llenan casi todo
const FPS_RADIO: u32 = 15;
const RESINCRONIZAR: Duration = Duration::from_secs(5);
/// más desfasado que esto con mpvpaper, se vuelve a arrancar ffmpeg
const DESFASE_MAX: f64 = 0.3;
const REVISAR: Duration = Duration::from_secs(1);
const REVISAR_ENLACE: Duration = Duration::from_secs(3);
/// cuánto se espera a mpvpaper al entrar a la sesión
const ESPERA_MPVPAPER: Duration = Duration::from_secs(20);
/// parejo entre cuadros: 0 nada, 1 se queda quieto. Con más cuadros hace falta
/// menos, y mucho hace que el movimiento se arrastre
const SUAVE: f32 = 0.25;

fn runtime() -> PathBuf {
    PathBuf::from(std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into()))
}

fn archivo_pid() -> PathBuf {
    runtime().join("chispa-video.pid")
}

/// Si está corriendo (para que `chispa hold` no le pinte encima).
pub fn activo() -> bool {
    std::fs::read_to_string(archivo_pid())
        .ok()
        .and_then(|p| p.trim().parse::<i32>().ok())
        .and_then(|p| std::fs::read_to_string(format!("/proc/{p}/comm")).ok())
        .is_some_and(|c| c.trim() == "chispa")
}

struct Mpv {
    pid: i32,
    video: PathBuf,
    socket: Option<PathBuf>,
}

/// Cuándo arrancó (en ticks), o None si se está muriendo.
fn arranque(pid: i32) -> Option<u64> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let r: Vec<&str> = s.rsplit_once(')')?.1.split_whitespace().collect();
    if matches!(r.first(), Some(&"Z") | Some(&"X")) {
        return None;
    }
    r.get(19)?.parse().ok()
}

/// El mpvpaper que corre, su video y su socket (si se lanzó con input-ipc-server).
/// Si hay dos (el viejo muriéndose y el nuevo), el más nuevo: pasó que se
/// agarraba el viejo, se moría al segundo, y el teclado se quedaba sin video.
fn buscar_mpvpaper() -> Option<Mpv> {
    let mut mejor: Option<(u64, Mpv)> = None;
    for e in std::fs::read_dir("/proc").ok()?.flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<i32>() else { continue };
        if std::fs::read_to_string(e.path().join("comm")).ok().as_deref().map(str::trim) != Some("mpvpaper") {
            continue;
        }
        let Ok(cmd) = std::fs::read(e.path().join("cmdline")) else { continue };
        let args: Vec<String> = cmd.split(|b| *b == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).into_owned()).collect();
        let Some(video) = args.last().map(PathBuf::from) else { continue };
        let socket = args
            .iter()
            .flat_map(|a| a.split_whitespace())
            .find_map(|t| t.trim_start_matches('-').strip_prefix("input-ipc-server=").map(PathBuf::from));
        let Some(cuando) = arranque(pid) else { continue };
        if mejor.as_ref().is_none_or(|(m, _)| cuando > *m) {
            mejor = Some((cuando, Mpv { pid, video, socket }));
        }
    }
    mejor.map(|(_, m)| m)
}

pub fn hay_mpvpaper() -> bool {
    buscar_mpvpaper().is_some()
}

fn vivo(pid: i32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|c| c.trim() == "mpvpaper")
}

/// Pausado a la fuerza (SIGSTOP, como hace el modo juego).
fn parado(pid: i32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| s.rsplit_once(')').is_some_and(|(_, r)| r.trim_start().starts_with('T')))
}

/// Dónde va mpv, en segundos, por su socket JSON.
fn posicion(socket: &PathBuf) -> Option<f64> {
    let mut s = UnixStream::connect(socket).ok()?;
    s.set_read_timeout(Some(Duration::from_millis(300))).ok()?;
    s.write_all(b"{\"command\":[\"get_property\",\"time-pos\"],\"request_id\":7}\n").ok()?;
    let mut todo = String::new();
    let mut b = [0u8; 1024];
    let fin = Instant::now() + Duration::from_millis(400);
    while Instant::now() < fin {
        let n = s.read(&mut b).ok()?;
        if n == 0 {
            break;
        }
        todo.push_str(&String::from_utf8_lossy(&b[..n]));
        for linea in todo.lines() {
            if linea.contains("\"request_id\":7") {
                let d = linea.split("\"data\":").nth(1)?;
                return d.split([',', '}']).next()?.trim().parse().ok();
            }
        }
    }
    None
}

fn duracion(video: &PathBuf) -> Option<f64> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1"])
        .arg(video)
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

fn hay_vaapi() -> bool {
    std::path::Path::new("/dev/dri/renderD128").exists()
}

/// ffmpeg decodificando desde `desde`, al paso del video (-re), en bucle.
struct Tubo {
    hijo: Child,
    rx: Receiver<Vec<u8>>,
    desde: f64,
    fps: u32,
    cuadros: Arc<AtomicU64>,
}

impl Tubo {
    fn lanzar(video: &PathBuf, desde: f64, fps: u32, vaapi: bool) -> Result<Tubo, String> {
        let mut c = Command::new("ffmpeg");
        c.args(["-nostdin", "-v", "error"]);
        if vaapi {
            c.args(["-hwaccel", "vaapi", "-hwaccel_output_format", "vaapi"]);
        }
        c.args(["-re", "-stream_loop", "-1", "-ss", &format!("{desde:.3}"), "-i"]).arg(video);
        let filtro = if vaapi {
            format!("fps={fps},scale_vaapi=w=192:h=108,hwdownload,format=nv12,scale={ANCHO}:{ALTO}:flags=area,format=rgb24")
        } else {
            format!("fps={fps},scale={ANCHO}:{ALTO}:flags=area,format=rgb24")
        };
        c.args(["-an", "-vf", &filtro, "-f", "rawvideo", "-"]);
        let mut hijo = c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| format!("ffmpeg: {e}"))?;
        let mut salida = hijo.stdout.take().ok_or("ffmpeg: sin salida")?;
        let (tx, rx) = channel();
        let cuadros = Arc::new(AtomicU64::new(0));
        let cuenta = Arc::clone(&cuadros);
        std::thread::spawn(move || {
            let mut b = vec![0u8; CUADRO];
            while salida.read_exact(&mut b).is_ok() {
                cuenta.fetch_add(1, Ordering::SeqCst);
                if tx.send(b.clone()).is_err() {
                    break;
                }
            }
        });
        Ok(Tubo { hijo, rx, desde, fps, cuadros })
    }

    /// Dónde va, según los cuadros que ya salieron.
    fn posicion(&self, dur: f64) -> f64 {
        (self.desde + self.cuadros.load(Ordering::SeqCst) as f64 / self.fps as f64) % dur.max(0.1)
    }
}

impl Drop for Tubo {
    fn drop(&mut self) {
        let _ = self.hijo.kill();
        let _ = self.hijo.wait();
    }
}

/// El color de cada tecla: el promedio del pedazo de video que le queda encima.
fn de_teclas(cuadro: &[u8], teclas: &[(u8, f32, f32)]) -> Vec<(u8, [f32; 3])> {
    teclas
        .iter()
        .map(|&(led, x, y)| {
            let cx = (x * ANCHO as f32) as isize;
            let cy = (y * ALTO as f32) as isize;
            let (mut s, mut n) = ([0f32; 3], 0f32);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (px, py) = (cx + dx, cy + dy);
                    if px < 0 || py < 0 || px >= ANCHO as isize || py >= ALTO as isize {
                        continue;
                    }
                    let o = (py as usize * ANCHO + px as usize) * 3;
                    for c in 0..3 {
                        s[c] += cuadro[o + c] as f32;
                    }
                    n += 1.0;
                }
            }
            (led, s.map(|v| v / n.max(1.0)))
        })
        .collect()
}

/// Una imagen (png, jpg, gif: lo que lea ffmpeg) tecla por tecla, igual que un
/// cuadro de video.
pub fn mapa_de_imagen(ruta: &std::path::Path) -> Option<luz::Mapa> {
    let out = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(ruta)
        .args(["-frames:v", "1", "-vf", &format!("scale={ANCHO}:{ALTO}:flags=area,format=rgb24"), "-f", "rawvideo", "-"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if out.stdout.len() < CUADRO {
        return None;
    }
    let teclas = crate::tui::posiciones();
    Some(de_teclas(&out.stdout[..CUADRO], &teclas).into_iter().map(|(led, c)| (led, realzar(c))).collect())
}

/// Los LEDs se ven deslavados con colores de video: más saturación, mismo tono.
fn realzar(c: [f32; 3]) -> Rgb {
    let rgb = (c[0].round() as u8, c[1].round() as u8, c[2].round() as u8);
    let (h, s, v) = luz::rgb_a_hsv(rgb);
    luz::hsv_a_rgb(h, (s * 1.4).min(1.0), v)
}

/// Cuántos cuadros salen de verdad por segundo y cuánto tarda mandar uno, en
/// XDG_RUNTIME_DIR/chispa-video.stat (lo enseña la TUI).
struct Medida {
    desde: Instant,
    cuadros: u32,
    tardado: Duration,
}

impl Medida {
    fn new() -> Medida {
        Medida { desde: Instant::now(), cuadros: 0, tardado: Duration::ZERO }
    }

    fn contar(&mut self, d: Duration, enlace: Enlace) {
        self.cuadros += 1;
        self.tardado += d;
        let pasado = self.desde.elapsed();
        if pasado >= Duration::from_secs(3) {
            let fps = self.cuadros as f64 / pasado.as_secs_f64();
            let ms = self.tardado.as_secs_f64() * 1000.0 / self.cuadros.max(1) as f64;
            let _ = std::fs::write(archivo_medida(), format!("{} {fps:.1} {ms:.1}\n", enlace.nombre()));
            *self = Medida::new();
        }
    }
}

fn archivo_medida() -> PathBuf {
    runtime().join("chispa-video.stat")
}

/// (enlace, cuadros por segundo, ms por cuadro) de lo que está corriendo
pub fn medida() -> Option<(String, f64, f64)> {
    if !activo() {
        return None;
    }
    let t = std::fs::read_to_string(archivo_medida()).ok()?;
    let mut p = t.trim().rsplitn(3, ' ');
    let ms = p.next()?.parse().ok()?;
    let fps = p.next()?.parse().ok()?;
    Some((p.next()?.to_string(), fps, ms))
}

static PARAR: AtomicBool = AtomicBool::new(false);

extern "C" fn al_parar(_: libc::c_int) {
    PARAR.store(true, Ordering::SeqCst);
}

fn candado(esperar: bool) -> Result<Option<File>, String> {
    let ruta = runtime().join("chispa-video.lock");
    let f = OpenOptions::new().create(true).truncate(false).write(true).open(&ruta).map_err(|e| format!("{}: {e}", ruta.display()))?;
    let fin = Instant::now() + Duration::from_secs(3);
    loop {
        // SAFETY: f sigue abierto; flock solo recibe su descriptor.
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(Some(f));
        }
        if !esperar || Instant::now() > fin {
            return Ok(None);
        }
        sleep(Duration::from_millis(100));
    }
}

/// Lo lanza el gancho del wallpaper, en el fondo.
pub fn lanzar_fondo() {
    if let Ok(yo) = std::env::current_exe() {
        let _ = Command::new(yo)
            .args(["video", "--auto"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn();
    }
}

pub fn correr(args: &[String]) -> Result<(), String> {
    let auto = match args.first().map(String::as_str) {
        None => false,
        Some("--auto") => true,
        Some("stop") => {
            if let Some(p) = std::fs::read_to_string(archivo_pid()).ok().and_then(|p| p.trim().parse::<i32>().ok())
                && activo()
            {
                // SAFETY: solo una señal a un pid que es chispa.
                unsafe { libc::kill(p, libc::SIGTERM) };
            }
            return Ok(());
        }
        Some(o) => return Err(format!("{}: {o} (chispa help)", l("no conozco esa opción", "unknown option"))),
    };
    if auto && !sigue_wallpaper() {
        return Ok(());
    }
    let Some(_uno) = candado(auto)? else {
        if !auto {
            println!("{}", l("ya está corriendo", "already running"));
        }
        return Ok(());
    };
    // al entrar a la sesión mpvpaper arranca con un sleep: se le espera
    let fin = Instant::now() + if auto { ESPERA_MPVPAPER } else { Duration::ZERO };
    let mpv = loop {
        if let Some(m) = buscar_mpvpaper() {
            break m;
        }
        if Instant::now() >= fin {
            return if auto { Ok(()) } else { Err(l("no hay un wallpaper en video (mpvpaper)", "no video wallpaper running (mpvpaper)")) };
        }
        sleep(Duration::from_millis(500));
    };
    std::fs::write(archivo_pid(), std::process::id().to_string()).map_err(|e| e.to_string())?;
    // SAFETY: el manejador solo toca un atómico.
    unsafe {
        libc::signal(libc::SIGTERM, al_parar as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, al_parar as *const () as libc::sighandler_t);
        libc::signal(libc::SIGHUP, al_parar as *const () as libc::sighandler_t);
    }
    let r = bucle(mpv, auto);
    let _ = std::fs::remove_file(archivo_pid());
    let _ = std::fs::remove_file(archivo_medida());
    r
}

/// mpvpaper se fue: se espera a que llegue otro (cambiar de un video a otro)
const ESPERA_OTRO: Duration = Duration::from_secs(6);
/// sin cuadro nuevo por este rato, se repite el último: si no, el teclado
/// regresa a lo guardado y se ve un parpadeo de otro color
const REPETIR: Duration = Duration::from_millis(700);

fn bucle(mut mpv: Mpv, auto: bool) -> Result<(), String> {
    let mut dur = duracion(&mpv.video).ok_or_else(|| format!("ffprobe: {}", mpv.video.display()))?;
    let vaapi = hay_vaapi();
    let teclas = crate::tui::posiciones();
    let mut t: Option<Teclado> = None;
    let mut tubo: Option<Tubo> = None;
    // ffmpeg tarda en dar su primer cuadro: se arranca así de adelantado
    let mut adelanto = 0.4f64;
    let mut lanzado = Instant::now();
    let mut primero = true;
    let (mut revisado, mut sincronizado, mut buscado) = (Instant::now(), Instant::now(), Instant::now() - REVISAR_ENLACE);
    let mut antes: HashMap<u8, [f32; 3]> = HashMap::new();
    let mut ultimo: Vec<(Rgb, Vec<u8>)> = Vec::new();
    let mut enviado = Instant::now();
    let mut medida = Medida::new();
    if !auto {
        println!("{}", l("siguiendo el video; Ctrl+C para parar", "following the video; Ctrl+C to stop"));
    }
    while !PARAR.load(Ordering::SeqCst) {
        if revisado.elapsed() >= REVISAR {
            revisado = Instant::now();
            // otro wallpaper, se quitó el video, o ya no se sigue al wallpaper
            if !sigue_wallpaper() {
                crate::anotar("ya no se sigue al wallpaper");
                return Ok(());
            }
            if !vivo(mpv.pid) {
                // otro video: se le espera repitiendo el último cuadro
                tubo = None;
                let fin = Instant::now() + ESPERA_OTRO;
                let nuevo = loop {
                    if let Some(m) = buscar_mpvpaper().filter(|m| m.pid != mpv.pid) {
                        break Some(m);
                    }
                    if Instant::now() >= fin || PARAR.load(Ordering::SeqCst) {
                        break None;
                    }
                    if let Some(n) = &t
                        && !ultimo.is_empty()
                    {
                        let _ = n.cuadro(&ultimo);
                    }
                    sleep(Duration::from_millis(400));
                };
                let Some(m) = nuevo else {
                    crate::anotar("se quitó el video");
                    return Ok(());
                };
                let Some(d) = duracion(&m.video) else {
                    crate::anotar(&format!("ffprobe no leyó {}", m.video.display()));
                    return Ok(());
                };
                crate::anotar(&format!("otro video: {}", m.video.display()));
                (mpv, dur) = (m, d);
                antes.clear();
                continue;
            }
            if crate::binario_viejo() {
                // ffmpeg primero: con exec no correría su Drop
                drop(tubo.take());
                drop(t.take());
                crate::reiniciar();
            }
        }
        // sin teclado, o llegó el cable y se anda por radio: se vuelve a buscar
        let cambiar = t.as_ref().is_none_or(|n| n.enlace() != Enlace::Cable && crate::hid::hay_cable());
        if cambiar && buscado.elapsed() >= REVISAR_ENLACE {
            buscado = Instant::now();
            if let Ok(n) = Teclado::abrir() {
                n.soltar();
                t = Some(n);
                tubo = None;
            }
        }
        // por cable no hay en vivo: se pausa hasta que regrese la radio
        if t.as_ref().is_some_and(|n| !n.en_vivo()) {
            tubo = None;
            let _ = std::fs::remove_file(archivo_medida());
            sleep(Duration::from_secs(1));
            continue;
        }
        let fps = FPS_RADIO;
        if parado(mpv.pid) {
            // pausado: se queda el último cuadro
            tubo = None;
            if let Some(n) = &t
                && !ultimo.is_empty()
                && enviado.elapsed() >= Duration::from_millis(800)
            {
                enviado = Instant::now();
                let _ = n.cuadro(&ultimo);
            }
            sleep(Duration::from_millis(200));
            continue;
        }
        let pos_mpv = || mpv.socket.as_ref().and_then(posicion);
        if tubo.is_none() {
            let desde = (pos_mpv().unwrap_or(0.0) + adelanto) % dur;
            tubo = Some(Tubo::lanzar(&mpv.video, desde, fps, vaapi)?);
            lanzado = Instant::now();
            primero = true;
            sincronizado = Instant::now();
        }
        if sincronizado.elapsed() >= RESINCRONIZAR {
            sincronizado = Instant::now();
            if let (Some(p), Some(tb)) = (pos_mpv(), &tubo) {
                let d = (tb.posicion(dur) - p).rem_euclid(dur);
                let desfase = d.min(dur - d);
                if desfase > DESFASE_MAX {
                    crate::anotar(&format!("desfasado {desfase:.2} s de mpvpaper: vuelvo a lanzar ffmpeg"));
                    tubo = None;
                    continue;
                }
            }
        }
        let Some(tb) = &tubo else { continue };
        let cuadro = match tb.rx.recv_timeout(Duration::from_millis(500)) {
            Ok(c) => tb.rx.try_iter().last().unwrap_or(c),
            Err(RecvTimeoutError::Timeout) => {
                if let Some(n) = &t
                    && !ultimo.is_empty()
                    && enviado.elapsed() >= REPETIR
                {
                    enviado = Instant::now();
                    let _ = n.cuadro(&ultimo);
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => {
                // ffmpeg se murió (¿sin VAAPI?): de nuevo en un rato
                crate::anotar("ffmpeg se murió: lo vuelvo a lanzar");
                tubo = None;
                if let Some(n) = &t
                    && !ultimo.is_empty()
                {
                    let _ = n.cuadro(&ultimo);
                }
                sleep(Duration::from_secs(1));
                continue;
            }
        };
        if primero {
            primero = false;
            adelanto = lanzado.elapsed().as_secs_f64().clamp(0.1, 2.0);
        }
        let Some(n) = &t else { continue };
        let mut colores = Vec::with_capacity(teclas.len());
        for (led, c) in de_teclas(&cuadro, &teclas) {
            let p = antes.get(&led).copied().unwrap_or(c);
            let s = [0, 1, 2].map(|i| p[i] * SUAVE + c[i] * (1.0 - SUAVE));
            antes.insert(led, s);
            colores.push((led, realzar(s)));
        }
        let grupos = luz::grupos(&colores, n.enlace() != Enlace::Cable);
        let antes_de_mandar = Instant::now();
        if n.cuadro(&grupos).is_err() {
            t = None;
            continue;
        }
        medida.contar(antes_de_mandar.elapsed(), n.enlace());
        ultimo = grupos;
        enviado = Instant::now();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cada_tecla_lee_su_pedazo() {
        // mitad izquierda roja, derecha azul
        let mut c = vec![0u8; CUADRO];
        for y in 0..ALTO {
            for x in 0..ANCHO {
                let o = (y * ANCHO + x) * 3;
                if x < ANCHO / 2 { c[o] = 255 } else { c[o + 2] = 255 }
            }
        }
        let t = de_teclas(&c, &[(1, 0.05, 0.5), (2, 0.95, 0.5)]);
        assert!(t[0].1[0] > 200.0 && t[0].1[2] < 10.0);
        assert!(t[1].1[2] > 200.0 && t[1].1[0] < 10.0);
    }

    #[test]
    fn las_teclas_quedan_dentro_del_cuadro() {
        let p = crate::tui::posiciones();
        assert_eq!(p.len(), luz::TECLAS.len());
        assert!(p.iter().all(|&(_, x, y)| (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)));
    }
}
