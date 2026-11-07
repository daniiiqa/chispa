//! `chispa video`: con un wallpaper en video (mpvpaper), el teclado es una
//! pantallita del wallpaper: cada tecla toma el color del pedazo de video que le
//! queda encima. El video se achica una vez (48×18 a 15 cuadros por segundo,
//! con ffmpeg por la GPU) y se guarda en ~/.cache/chispa/video; de ahí solo se
//! lee el cuadro que toca según dónde va mpvpaper, que se le pregunta por su
//! socket. No queda nada decodificando.
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
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

const ANCHO: usize = 48;
const ALTO: usize = 18;
const CUADRO: usize = ANCHO * ALTO * 3;
/// por radio un cuadro de 10 colores son 9 paquetes, cada uno ~7 ms (5 de
/// aire, más pegados se pierden): medido, 63 ms por cuadro. El techo es ~15;
/// con menos colores no baja, las 80 teclas ya llenan casi todo
const FPS: u32 = 15;
/// cada cuánto se le pregunta a mpv dónde va (entre preguntas se cuenta el reloj)
const CONSULTAR: Duration = Duration::from_secs(2);
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

fn hay_vaapi() -> bool {
    std::path::Path::new("/dev/dri/renderD128").exists()
}

/// El video achicado (48×18 a 15 cuadros por segundo, ~2.6 KB por cuadro), en
/// ~/.cache/chispa/video. Se saca una vez por video y de ahí solo se lee: antes
/// ffmpeg decodificaba el 4K en vivo todo el tiempo, con ~370 MB de búferes de
/// la GPU, y como nunca quedaba en sincronía se relanzaba cada 5 s.
fn cache(video: &std::path::Path) -> Option<PathBuf> {
    let m = std::fs::metadata(video).ok()?;
    let cambio = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let nombre = video.file_name()?.to_string_lossy();
    let base = std::env::var("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache"));
    Some(base.join("chispa/video").join(format!("{nombre}-{}-{cambio}-{FPS}fps.rgb", m.len())))
}

/// Saca el video achicado (por la GPU si se puede), con prioridad baja.
fn construir(video: &std::path::Path, destino: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(destino.parent().ok_or("cache")?).map_err(|e| e.to_string())?;
    let tmp = destino.with_extension("tmp");
    let filtro = |vaapi: bool| {
        if vaapi {
            format!("fps={FPS},scale_vaapi=w=192:h=108,hwdownload,format=nv12,scale={ANCHO}:{ALTO}:flags=area,format=rgb24")
        } else {
            format!("fps={FPS},scale={ANCHO}:{ALTO}:flags=area,format=rgb24")
        }
    };
    for vaapi in [hay_vaapi(), false] {
        let mut c = Command::new("ffmpeg");
        c.args(["-nostdin", "-v", "error", "-y"]);
        if vaapi {
            c.args(["-hwaccel", "vaapi", "-hwaccel_output_format", "vaapi"]);
        }
        c.arg("-i").arg(video).args(["-an", "-vf", &filtro(vaapi), "-f", "rawvideo"]).arg(&tmp);
        let mut hijo = c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|e| format!("ffmpeg: {e}"))?;
        // SAFETY: solo baja la prioridad de nuestro hijo.
        unsafe { libc::setpriority(libc::PRIO_PROCESS, hijo.id(), 10) };
        let ok = loop {
            if PARAR.load(Ordering::SeqCst) {
                let _ = hijo.kill();
                let _ = hijo.wait();
                let _ = std::fs::remove_file(&tmp);
                return Err("parado".into());
            }
            match hijo.try_wait() {
                Ok(Some(st)) => break st.success(),
                Ok(None) => sleep(Duration::from_millis(200)),
                Err(e) => return Err(e.to_string()),
            }
        };
        if ok && std::fs::metadata(&tmp).is_ok_and(|m| m.len() >= CUADRO as u64) {
            return std::fs::rename(&tmp, destino).map_err(|e| e.to_string());
        }
        let _ = std::fs::remove_file(&tmp);
    }
    Err(format!("ffmpeg no pudo achicar {}", video.display()))
}

/// Los cuadros del video achicado, sacándolos si no están.
fn cuadros_de(video: &std::path::Path) -> Result<Vec<u8>, String> {
    let c = cache(video).ok_or_else(|| format!("{}: no se puede leer", video.display()))?;
    if !c.exists() {
        crate::anotar(&format!("achicando {} (una vez)…", video.display()));
        let t0 = Instant::now();
        construir(video, &c)?;
        crate::anotar(&format!("listo en {:.1} s", t0.elapsed().as_secs_f64()));
    }
    let mut b = std::fs::read(&c).map_err(|e| format!("{}: {e}", c.display()))?;
    b.truncate(b.len() / CUADRO * CUADRO);
    if b.is_empty() {
        return Err(format!("{}: vacío", c.display()));
    }
    Ok(b)
}

/// Dónde va mpv, contando el tiempo entre preguntas. Se le pregunta de vez en
/// cuando; si no avanzó entre dos preguntas, está en pausa.
struct Reloj {
    pos: f64,
    desde: Instant,
    pausado: bool,
    preguntado: Instant,
}

impl Reloj {
    fn new(socket: Option<&PathBuf>) -> Reloj {
        let pos = socket.and_then(posicion).unwrap_or(0.0);
        Reloj { pos, desde: Instant::now(), pausado: false, preguntado: Instant::now() }
    }

    fn ahora(&mut self, socket: Option<&PathBuf>) -> f64 {
        if self.preguntado.elapsed() >= CONSULTAR {
            self.preguntado = Instant::now();
            if let Some(p) = socket.and_then(posicion) {
                self.pausado = (p - self.pos).abs() < 0.01;
                self.pos = p;
                self.desde = Instant::now();
            }
        }
        if self.pausado { self.pos } else { self.pos + self.desde.elapsed().as_secs_f64() }
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

/// Lo que se está viendo, para que la TUI lo dibuje (se escribe 4 veces por segundo).
fn archivo_vivo() -> PathBuf {
    runtime().join("chispa-video.vivo")
}

pub fn mapa_vivo() -> Option<luz::Mapa> {
    if !activo() {
        return None;
    }
    crate::sostener::de_texto(&std::fs::read_to_string(archivo_vivo()).ok()?)
}

/// Parar o volver a seguir el video (el botón de la TUI).
pub fn parar() {
    if let Some(p) = std::fs::read_to_string(archivo_pid()).ok().and_then(|p| p.trim().parse::<i32>().ok())
        && activo()
    {
        // SAFETY: solo una señal a un pid que es chispa.
        unsafe { libc::kill(p, libc::SIGTERM) };
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
            parar();
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
    // SAFETY: el manejador solo toca un atómico.
    unsafe {
        libc::signal(libc::SIGTERM, al_parar as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, al_parar as *const () as libc::sighandler_t);
        libc::signal(libc::SIGHUP, al_parar as *const () as libc::sighandler_t);
    }
    let r = bucle(mpv, auto);
    tocando(false);
    r
}

/// mpvpaper se fue: se espera a que llegue otro (cambiar de un video a otro)
const ESPERA_OTRO: Duration = Duration::from_secs(6);

/// El cuadro que toca a los `pos` segundos: mpv da la vuelta al video y su
/// reloj también, pero entre preguntas se cuenta de más, así que se envuelve.
fn indice(pos: f64, total: usize) -> usize {
    ((pos.max(0.0) * FPS as f64) as usize) % total.max(1)
}

/// Mientras corre, `chispa hold` no pinta: el pid se anota solo al tocar.
fn tocando(si: bool) {
    if si {
        let _ = std::fs::write(archivo_pid(), std::process::id().to_string());
    } else {
        let _ = std::fs::remove_file(archivo_pid());
        let _ = std::fs::remove_file(archivo_medida());
        let _ = std::fs::remove_file(archivo_vivo());
    }
}

fn bucle(mut mpv: Mpv, auto: bool) -> Result<(), String> {
    let teclas = crate::tui::posiciones();
    // mientras se saca el cache, lo sostenido se sigue viendo
    let mut cuadros = cuadros_de(&mpv.video)?;
    let mut reloj = Reloj::new(mpv.socket.as_ref());
    let mut t: Option<Teclado> = None;
    let (mut revisado, mut buscado) = (Instant::now(), Instant::now() - REVISAR_ENLACE);
    let mut antes: HashMap<u8, [f32; 3]> = HashMap::new();
    let mut ultimo: Vec<(Rgb, Vec<u8>)> = Vec::new();
    let mut medida = Medida::new();
    let mut escrito = Instant::now();
    let paso = Duration::from_secs_f64(1.0 / FPS as f64);
    let mut siguiente = Instant::now();
    tocando(true);
    if !auto {
        println!("{}", l("siguiendo el video; Ctrl+C para parar", "following the video; Ctrl+C to stop"));
    }
    while !PARAR.load(Ordering::SeqCst) {
        if revisado.elapsed() >= REVISAR {
            revisado = Instant::now();
            if !sigue_wallpaper() {
                crate::anotar("ya no se sigue al wallpaper");
                return Ok(());
            }
            if crate::binario_viejo() {
                drop(t.take());
                crate::reiniciar();
            }
            if !vivo(mpv.pid) {
                // otro video: se le espera repitiendo el último cuadro
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
                crate::anotar(&format!("otro video: {}", m.video.display()));
                tocando(false);
                cuadros = cuadros_de(&m.video)?;
                reloj = Reloj::new(m.socket.as_ref());
                mpv = m;
                antes.clear();
                tocando(true);
                continue;
            }
        }
        // sin teclado, o llegó el cable y se anda por radio: se vuelve a buscar
        let cambiar = t.as_ref().is_none_or(|n| n.enlace() != Enlace::Cable && crate::hid::hay_cable());
        if cambiar && buscado.elapsed() >= REVISAR_ENLACE {
            buscado = Instant::now();
            if let Ok(n) = Teclado::abrir() {
                n.soltar();
                t = Some(n);
            }
        }
        // al paso del video, sin amontonar
        let ahora = Instant::now();
        if ahora < siguiente {
            sleep(siguiente - ahora);
        }
        siguiente = Instant::now().max(siguiente) + paso;
        let Some(n) = &t else { continue };
        // por cable no hay en vivo: nada hasta que regrese la radio
        if !n.en_vivo() {
            let _ = std::fs::remove_file(archivo_medida());
            sleep(Duration::from_secs(1));
            continue;
        }
        let total = cuadros.len() / CUADRO;
        let pos = if parado(mpv.pid) { reloj.pos } else { reloj.ahora(mpv.socket.as_ref()) };
        let i = indice(pos, total);
        let cuadro = &cuadros[i * CUADRO..(i + 1) * CUADRO];
        let mut colores = Vec::with_capacity(teclas.len());
        for (led, c) in de_teclas(cuadro, &teclas) {
            let p = antes.get(&led).copied().unwrap_or(c);
            let s = [0, 1, 2].map(|i| p[i] * SUAVE + c[i] * (1.0 - SUAVE));
            antes.insert(led, s);
            colores.push((led, realzar(s)));
        }
        let grupos = luz::grupos(&colores, true);
        let t0 = Instant::now();
        if n.cuadro(&grupos).is_err() {
            t = None;
            continue;
        }
        medida.contar(t0.elapsed(), n.enlace());
        if escrito.elapsed() >= Duration::from_millis(250) {
            escrito = Instant::now();
            let _ = std::fs::write(archivo_vivo(), crate::sostener::a_texto(&colores));
        }
        ultimo = grupos;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_cuadro_da_la_vuelta_con_el_video() {
        assert_eq!(indice(0.0, 100), 0);
        assert_eq!(indice(1.0, 100), FPS as usize);
        // pasado el final (el reloj contó de más) regresa al principio
        assert_eq!(indice(100.0 / FPS as f64, 100), 0);
        assert_eq!(indice(-1.0, 100), 0);
    }

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
