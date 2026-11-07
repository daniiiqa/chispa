//! `chispa hold`: el color nuevo se ve luego luego, en vivo, y se guarda
//! después, cuando nadie está viendo el teclado. Guardar siempre apaga las
//! luces un momento (probado por radio y por cable, con tabla, ajustes y
//! paletas), así que en vez de guardar al cambiar de color se sostiene en vivo
//! y se guarda:
//!   - al bloquear la pantalla o antes de suspender (`chispa save` desde hypridle),
//!   - al salir de la sesión o apagar (SIGTERM / SIGHUP).
//!
//! Probado por 2.4 GHz: los cuadros no lo mantienen despierto (se duerme a su
//! hora) y al despertarlo sale directo el color sostenido. Así que no se guarda
//! antes de dormirse: eso parpadeaba con uno ahí sentado. Si se pide guardar y
//! está dormido, queda pendiente y se guarda cuando despierte.
//!
//! Guarda una vez y se sale. El color va en un archivo de XDG_RUNTIME_DIR; si
//! alguien lo borra, se sale sin guardar.

use crate::hid::Teclado;
use crate::luz::{self, Mapa, Rgb};
use crate::l;
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

/// cada cuánto se repite el cuadro: por radio se regresa a lo guardado como a los
/// 2 s, y los cuadros no llevan acuse: con 500 ms a veces se perdían varios
/// seguidos (tecleando) y se asomaba lo guardado
const ENTRE_CUADROS: Duration = Duration::from_millis(200);
const REVISAR_ENLACE: Duration = Duration::from_secs(3);
/// cuánto se espera a que el de fondo guarde y se salga
const ESPERA_SALIDA: Duration = Duration::from_secs(25);

fn runtime() -> PathBuf {
    PathBuf::from(std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into()))
}

fn archivo_color() -> PathBuf {
    runtime().join("chispa-hold")
}

fn archivo_pid() -> PathBuf {
    runtime().join("chispa-hold.pid")
}

/// Un renglón por tecla: "led #rrggbb". Un solo "#rrggbb" es todo el teclado.
fn leer_mapa() -> Option<Mapa> {
    de_texto(&std::fs::read_to_string(archivo_color()).ok()?)
}

pub(crate) fn de_texto(t: &str) -> Option<Mapa> {
    let mut m = Mapa::new();
    for r in t.lines().map(str::trim).filter(|r| !r.is_empty()) {
        match r.split_once(' ') {
            Some((led, c)) => m.push((led.parse().ok()?, luz::color(c)?)),
            None => return Some(luz::mapa_uniforme(luz::color(r)?)),
        }
    }
    (!m.is_empty()).then_some(m)
}

fn escribir_mapa(m: &Mapa) -> Result<(), String> {
    let f = archivo_color();
    let tmp = f.with_extension("tmp");
    std::fs::write(&tmp, a_texto(m)).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &f).map_err(|e| format!("{}: {e}", f.display()))
}

pub(crate) fn a_texto(m: &Mapa) -> String {
    m.iter().map(|(led, (r, g, b))| format!("{led} #{r:02x}{g:02x}{b:02x}\n")).collect()
}

/// El de fondo, si está corriendo (y es chispa, no otro con el mismo pid).
fn pid_vivo() -> Option<i32> {
    let pid: i32 = std::fs::read_to_string(archivo_pid()).ok()?.trim().parse().ok()?;
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    (comm.trim() == "chispa").then_some(pid)
}

/// Lo que se está sosteniendo sin guardar.
pub fn sostenido() -> Option<Mapa> {
    pid_vivo().and(leer_mapa())
}

/// Sostener este mapa: si no hay uno de fondo, se lanza.
pub fn sostener(m: &Mapa) -> Result<(), String> {
    escribir_mapa(m)?;
    if pid_vivo().is_some() {
        return Ok(());
    }
    let yo = std::env::current_exe().map_err(|e| e.to_string())?;
    Command::new(yo)
        .arg("hold")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("chispa hold: {e}"))
}

/// Termina lo sostenido. Con `guardar`, el de fondo lo guarda antes de salirse
/// (parpadea); sin, solo se va y regresa el color que tenía. Hay que llamarlo
/// antes de abrir el teclado: el de fondo necesita el candado para guardar.
pub fn terminar(guardar: bool) -> Option<Mapa> {
    let pid = pid_vivo()?;
    let c = leer_mapa();
    if guardar {
        // SAFETY: solo manda una señal a un pid que es chispa.
        unsafe { libc::kill(pid, libc::SIGUSR1) };
    } else {
        let _ = std::fs::remove_file(archivo_color());
    }
    let fin = Instant::now() + ESPERA_SALIDA;
    while pid_vivo().is_some() && Instant::now() < fin {
        sleep(Duration::from_millis(100));
    }
    c
}

static GUARDAR: AtomicBool = AtomicBool::new(false);
/// de salida (apagar, salir de la sesión): guardar sí o sí y salirse
static SALIR: AtomicBool = AtomicBool::new(false);

extern "C" fn al_pedir(_: libc::c_int) {
    GUARDAR.store(true, Ordering::SeqCst);
}

extern "C" fn al_salir(_: libc::c_int) {
    GUARDAR.store(true, Ordering::SeqCst);
    SALIR.store(true, Ordering::SeqCst);
}

fn solo_uno() -> Result<Option<File>, String> {
    let ruta = runtime().join("chispa-hold.lock");
    let f = OpenOptions::new().create(true).truncate(false).write(true).open(&ruta).map_err(|e| format!("{}: {e}", ruta.display()))?;
    // SAFETY: f sigue abierto; flock solo recibe su descriptor.
    Ok((unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0).then_some(f))
}

fn cuadro(t: &Teclado, m: &Mapa) -> Result<(), String> {
    t.cuadro(&luz::grupos(m, t.enlace() != crate::hid::Enlace::Cable))
}

/// Guarda de una vez, sin fundir: ya se está viendo. Por radio el cuadro en
/// vivo lleva 10 colores; lo guardado, cada tecla con el suyo.
fn guardar(t: &Teclado, m: &Mapa) -> Result<(), String> {
    t.tomar();
    t.guardar_sin_fundir();
    t.ver_como(Some(m.clone()));
    let teclas: Vec<(usize, Rgb)> = m.iter().map(|(led, c)| (*led as usize, *c)).collect();
    let r = crate::aplicar_tabla(t, &teclas).map(|_| ());
    t.soltar();
    r
}

fn limpiar() {
    let _ = std::fs::remove_file(archivo_color());
    let _ = std::fs::remove_file(archivo_pid());
}

/// El proceso de fondo.
pub fn correr() -> Result<(), String> {
    let Some(_uno) = solo_uno()? else { return Ok(()) };
    std::fs::write(archivo_pid(), std::process::id().to_string()).map_err(|e| e.to_string())?;
    // SAFETY: los manejadores solo tocan atómicos.
    unsafe {
        libc::signal(libc::SIGUSR1, al_pedir as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, al_salir as *const () as libc::sighandler_t);
        libc::signal(libc::SIGHUP, al_salir as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, al_salir as *const () as libc::sighandler_t);
    }
    let r = bucle();
    limpiar();
    r
}

fn bucle() -> Result<(), String> {
    let mut t: Option<Teclado> = None;
    let mut buscado = Instant::now() - REVISAR_ENLACE;
    let mut fallos = 0;
    let mut revisado = Instant::now();
    loop {
        // lo borraron: alguien más ya se encargó
        let Some(c) = leer_mapa() else {
            crate::anotar("me quitaron lo sostenido: me voy sin guardar");
            return Ok(());
        };
        if revisado.elapsed() >= REVISAR_ENLACE {
            revisado = Instant::now();
            if crate::binario_viejo() {
                drop(t.take());
                crate::reiniciar();
            }
        }
        // sin teclado, o llegó el cable y se anda por radio: se vuelve a buscar
        let cambiar = t.as_ref().is_none_or(|n| n.enlace() != crate::hid::Enlace::Cable && crate::hid::hay_cable());
        if cambiar && buscado.elapsed() >= REVISAR_ENLACE {
            buscado = Instant::now();
            if let Ok(n) = Teclado::abrir() {
                n.soltar();
                t = Some(n);
            }
        }
        // llegó el cable: no se puede sostener en vivo, se guarda ya
        if t.as_ref().is_some_and(|n| !n.en_vivo()) && !GUARDAR.swap(true, Ordering::SeqCst) {
            crate::anotar("llegó el cable: guardo ya");
        }
        if GUARDAR.load(Ordering::SeqCst) {
            match t.as_ref().map(|n| guardar(n, &c)) {
                Some(Ok(())) => {
                    crate::anotar("guardado en el teclado");
                    return Ok(());
                }
                r => {
                    // una vez, no cada intento: dormido, esto se repite por horas
                    if let Some(Err(e)) = r
                        && fallos == 0
                    {
                        crate::anotar(&format!("no pude guardar (queda pendiente): {e}"));
                    }
                    // dormido o desconectado: queda pendiente y se vuelve a
                    // intentar al rato. Al salir de la sesión no hay más tiempo.
                    fallos += 1;
                    if SALIR.load(Ordering::SeqCst) && fallos >= 3 {
                        return Err(l("no pude guardar el color sostenido", "couldn't save the held color"));
                    }
                    t = None;
                }
            }
        }
        // el video del wallpaper manda mientras corre
        if let Some(n) = &t
            && !crate::video::activo()
            && cuadro(n, &c).is_err()
        {
            t = None;
        }
        sleep(ENTRE_CUADROS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_archivo_de_lo_sostenido_va_y_viene() {
        let m: Mapa = luz::TECLAS.iter().enumerate().map(|(i, k)| (k.1 as u8, (i as u8, 255 - i as u8, 7))).collect();
        assert_eq!(de_texto(&a_texto(&m)), Some(m));
        // el formato viejo, un solo color, también se entiende
        assert_eq!(de_texto("#102030\n"), Some(luz::mapa_uniforme((0x10, 0x20, 0x30))));
        // y lo roto no se sostiene
        assert_eq!(de_texto("3 #zzzzzz\n"), None);
        assert_eq!(de_texto(""), None);
    }
}
