//! Hablarle al teclado por hidraw: por cable, por el dongle o por Bluetooth. Lo
//! único que sale de aquí son los comandos de `Cmd`: leer y escribir la luz.
//! Por cable es el reporte 6, por radio el 0x13, y cada uno tiene sus números.
//! No hay forma de mandar otro reporte (el 5 puede meterlo al bootloader) ni
//! comandos de firmware, reset o teclas.

use crate::luz::{LARGO_LUZ, LEDS};
use crate::{l, radio};
use std::cell::{Cell, RefCell};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CABLE_ID: &str = "0003:0000258A:0000010C";
const DONGLE_ID: &str = ":00003554:0000FA09";
const BT_IDS: [&str; 2] = [":00003554:0000FA07", ":00003554:0000FA08"];
const REPORT: u8 = 6;
const LEN: usize = 520;
const HEADER: usize = 8;
/// aire entre comandos: una ráfaga de comandos pudo haber colgado al teclado
const PAUSA_LEER: Duration = Duration::from_millis(50);
const PAUSA_ESCRIBIR: Duration = Duration::from_millis(150);

/// por radio: lo que espera la app oficial entre paquetes
const PAUSA_PAQUETE: Duration = Duration::from_millis(15);
/// por radio: sin teclas abajo y sin teclear por este rato antes de hablarle
const QUIETO: Duration = Duration::from_millis(400);
const MAX_TECLA_ABAJO: Duration = Duration::from_secs(5);
const ESPERA_ECO: Duration = Duration::from_secs(1);
/// la primera respuesta tarda de 16 ms a más de 1 s: la radio se duerme entre mensajes
const ESPERA_RAFAGA: Duration = Duration::from_secs(2);
const FIN_RAFAGA: Duration = Duration::from_millis(250);
/// recién escrito, por Bluetooth se pierde todavía más
const REPOSO_BT: Duration = Duration::from_millis(500);
/// por radio, entre paquetes de un cuadro: más pegados se pierden o no se ven
const PAUSA_VIVO: Duration = Duration::from_millis(5);
/// un cuadro cada tanto al fundir
const PASO_FUNDIR: Duration = Duration::from_millis(40);

/// Los únicos comandos que existen para chispa.
#[derive(Clone, Copy, PartialEq)]
pub enum Cmd {
    LeerLuz,
    EscribirLuz,
    LeerTabla,
    EscribirTabla,
    /// solo por radio y solo lectura
    LeerBateria,
}

impl Cmd {
    /// por cable: reporte, comando, 0, 0, paquetes, índice, tamaño
    fn cabecera(self) -> [u8; HEADER] {
        match self {
            Cmd::LeerLuz => [REPORT, 0x84, 0, 0, 1, 0, 0x80, 0x00],
            Cmd::EscribirLuz => [REPORT, 0x04, 0, 0, 1, 0, 0x80, 0x00],
            Cmd::LeerTabla => [REPORT, 0x86, 0, 0, 1, 0, 0x80, 0x01],
            Cmd::EscribirTabla => [REPORT, 0x06, 0, 0, 1, 0, 0x7e, 0x01],
            // por cable no hay batería: leer() lo rechaza antes
            Cmd::LeerBateria => [0; HEADER],
        }
    }

    /// por radio: el comando y el largo exacto de su bloque
    fn radio(self) -> (u8, usize) {
        match self {
            Cmd::LeerLuz => (radio::LEER_LUZ, LARGO_LUZ),
            Cmd::EscribirLuz => (radio::ESCRIBIR_LUZ, LARGO_LUZ),
            Cmd::LeerTabla => (radio::LEER_TABLA, 3 * LEDS),
            Cmd::EscribirTabla => (radio::ESCRIBIR_TABLA, 3 * LEDS),
            Cmd::LeerBateria => (radio::LEER_BATERIA, 2),
        }
    }
}

const fn ioc_rw(nr: u64, len: usize) -> u64 {
    (3 << 30) | ((len as u64) << 16) | ((b'H' as u64) << 8) | nr
}
const SET_FEATURE: u64 = ioc_rw(0x06, LEN);
const GET_FEATURE: u64 = ioc_rw(0x07, LEN);

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Enlace {
    Cable,
    Dongle,
    Bluetooth,
}

impl Enlace {
    /// Si se le pueden mandar cuadros en vivo (ver Teclado::en_vivo).
    pub fn en_vivo(self) -> bool {
        self != Enlace::Cable
    }

    pub fn nombre(self) -> String {
        match self {
            Enlace::Cable => l("cable", "wired"),
            Enlace::Dongle => "2.4 GHz".into(),
            Enlace::Bluetooth => "Bluetooth".into(),
        }
    }
}

/// Qué enlace es este nodo, por su ID y por lo que declara su descriptor.
fn enlace_de(id: &str, desc: &[u8]) -> Option<Enlace> {
    let id = id.to_uppercase();
    if id == CABLE_ID {
        // 85 06 = reporte 6; 96 07 02 = 519 bytes de datos
        return (contiene(desc, &[0x85, 0x06]) && contiene(desc, &[0x96, 0x07, 0x02])).then_some(Enlace::Cable);
    }
    // página 0xff02, uso 2, colección, reporte 0x13
    if !contiene(desc, &[0x06, 0x02, 0xff, 0x09, 0x02, 0xa1, 0x01, 0x85, 0x13]) {
        return None;
    }
    if id.ends_with(DONGLE_ID) {
        Some(Enlace::Dongle)
    } else if BT_IDS.iter().any(|b| id.ends_with(b)) {
        Some(Enlace::Bluetooth)
    } else {
        None
    }
}

pub struct Teclado {
    f: File,
    enlace: Enlace,
    /// dos chispa a la vez no: el candado se suelta al terminar
    _candado: Rc<File>,
    colgado: Cell<bool>,
    /// los nodos del teclado, para ver si hay teclas abajo (por radio)
    teclas: Vec<File>,
    abajo: RefCell<Vec<bool>>,
    ultima_tecla: Cell<Instant>,
    ultima_escritura: Cell<Option<Instant>>,
    /// guardar sin fundir: lo que se ve ya es lo que se guarda (lo sostenido)
    sin_fundir: Cell<bool>,
    /// lo que se ve aunque no esté guardado (lo sostenido en vivo)
    visto: RefCell<Option<crate::luz::Mapa>>,
    /// la última vez que se repitió lo sostenido mientras se esperaba
    mantenido: Cell<Instant>,
}

fn candado() -> Result<File, String> {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    let ruta = PathBuf::from(dir).join("chispa.lock");
    let f = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&ruta)
        .map_err(|e| format!("{}: {e}", ruta.display()))?;
    // SAFETY: f sigue abierto; flock solo recibe su descriptor.
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(format!("{}: {}", ruta.display(), std::io::Error::last_os_error()));
    }
    Ok(f)
}

fn uevent(dev: &Path, clave: &str) -> Option<String> {
    fs::read_to_string(dev.join("device/uevent"))
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix(clave)?.strip_prefix('=').map(String::from))
}

fn contiene(h: &[u8], n: &[u8]) -> bool {
    h.windows(n.len()).any(|w| w == n)
}

/// Espera a que un descriptor tenga algo. false si se acabó el tiempo.
fn hay_datos(f: &File, espera: Duration) -> Result<bool, std::io::Error> {
    let mut p = libc::pollfd { fd: f.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    // SAFETY: p vive durante la llamada y se pasa uno solo.
    let r = unsafe { libc::poll(&mut p, 1, espera.as_millis() as libc::c_int) };
    if r < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(r > 0)
}

/// 0-50 ms, para no caer siempre en el mismo hueco de la radio.
fn al_azar() -> Duration {
    let n = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    Duration::from_millis(u64::from(n % 50))
}

fn abrir_nodo(ruta: &Path) -> Result<File, String> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(ruta)
        .map_err(|err| format!("{}: {err} ({})", ruta.display(), l("¿falta la regla de udev? mira el README", "is the udev rule missing? see the README")))
}

/// Repetir lo sostenido solo en la espera de ANTES de empezar (quieto > 0). Entre
/// paquetes de una escritura se espera con quieto = 0, y un cuadro ahí la echa a
/// perder (probado: el teclado guardó mal y chispa lo regresó).
fn puede_mantener(quieto: Duration) -> bool {
    !quieto.is_zero()
}

/// Si el cable está conectado (sin abrir nada): los de fondo se pasan a él.
pub fn hay_cable() -> bool {
    fs::read_dir("/sys/class/hidraw").is_ok_and(|d| d.flatten().any(|e| uevent(&e.path(), "HID_ID").is_some_and(|id| id.to_uppercase() == CABLE_ID)))
}

impl Teclado {
    /// El cable manda: si está conectado, el teclado está por ahí aunque el
    /// dongle siga puesto. Si hay dongle y Bluetooth, se usa el que conteste.
    pub fn abrir() -> Result<Teclado, String> {
        let mut nodos: Vec<(Enlace, String, PathBuf)> = Vec::new();
        let dir = fs::read_dir("/sys/class/hidraw").map_err(|e| format!("/sys/class/hidraw: {e}"))?;
        for e in dir.flatten() {
            let Some(id) = uevent(&e.path(), "HID_ID") else { continue };
            let desc = fs::read(e.path().join("device/report_descriptor")).unwrap_or_default();
            if let Some(enlace) = enlace_de(&id, &desc) {
                nodos.push((enlace, id, PathBuf::from("/dev").join(e.file_name())));
            }
        }
        nodos.sort_by_key(|n| match n.0 {
            Enlace::Cable => 0,
            Enlace::Dongle => 1,
            Enlace::Bluetooth => 2,
        });
        if nodos.is_empty() {
            return Err(l("no encontré el AULA F75 (ni por cable, ni dongle, ni Bluetooth)", "AULA F75 not found (no cable, dongle or Bluetooth)"));
        }
        let cerrojo = Rc::new(candado()?);
        let varios = nodos.len() > 1 && nodos[0].0 != Enlace::Cable;
        let mut ultimo_error = String::new();
        for (enlace, id, ruta) in nodos {
            let t = Teclado {
                f: abrir_nodo(&ruta)?,
                enlace,
                _candado: Rc::clone(&cerrojo),
                colgado: Cell::new(false),
                teclas: if enlace == Enlace::Cable { Vec::new() } else { nodos_de_teclas(&id) },
                abajo: RefCell::new(Vec::new()),
                ultima_tecla: Cell::new(Instant::now()),
                ultima_escritura: Cell::new(None),
                sin_fundir: Cell::new(false),
                visto: RefCell::new(None),
                mantenido: Cell::new(Instant::now()),
            };
            t.abajo.borrow_mut().resize(t.teclas.len(), false);
            if !varios {
                return Ok(t);
            }
            match t.leer(Cmd::LeerLuz, LARGO_LUZ) {
                Ok(_) => {
                    t.colgado.set(false);
                    return Ok(t);
                }
                Err(e) => ultimo_error = e,
            }
        }
        Err(ultimo_error)
    }

    /// El visualizador no se queda con el candado: si no, nadie más podría
    /// usar chispa mientras suena la música.
    pub fn soltar(&self) {
        // SAFETY: el archivo sigue abierto; flock solo recibe su descriptor.
        unsafe { libc::flock(self._candado.as_raw_fd(), libc::LOCK_UN) };
    }

    pub(crate) fn tomar(&self) {
        // SAFETY: el archivo sigue abierto; flock solo recibe su descriptor.
        unsafe { libc::flock(self._candado.as_raw_fd(), libc::LOCK_EX) };
    }

    /// Un cuadro en vivo. No se guarda en el teclado, y nadie confirma que llegó.
    pub fn cuadro(&self, grupos: &[crate::musica::Grupo]) -> Result<(), String> {
        if !self.en_vivo() {
            return Ok(());
        }
        self.tomar();
        let r = self.cuadro_radio(grupos);
        self.soltar();
        r
    }

    /// Por cable NO hay cuadros en vivo: el teclado tarda 66 ms en tomar cada
    /// uno, los pinta barriendo de izquierda a derecha y mientras tanto no lee
    /// las teclas (probado: con el video a 15 por segundo se comía lo que
    /// tecleabas). Y no hay forma de hacerlo más rápido: su endpoint de control
    /// es de 8 bytes y el reporte 6 siempre mide 520, así que son 65 paquetes
    /// de ~1 ms sin importar cuánto traiga (medido: completo, grupos sub 1 y
    /// sub 2, todos 66 ms). Por radio no pasa: va por el aire, no por el USB.
    pub fn en_vivo(&self) -> bool {
        self.enlace.en_vivo()
    }

    /// Un cuadro al momento y sin soltar el candado, para ver el color mientras
    /// se guarda. Por cable guardar es rápido y no hace falta.
    pub fn adelantar(&self, m: &crate::luz::Mapa) {
        if self.enlace != Enlace::Cable {
            let _ = self.cuadro_mapa(m);
        }
    }

    /// Un mapa en vivo y sin soltar el candado.
    fn cuadro_mapa(&self, m: &crate::luz::Mapa) -> Result<(), String> {
        if !self.en_vivo() {
            return Ok(());
        }
        self.cuadro_radio(&crate::luz::grupos(m, true))
    }

    /// De un mapa a otro con cuadros en vivo. No guarda nada: lo de antes sigue
    /// guardado hasta que se escriba.
    pub fn fundir(&self, de: &crate::luz::Mapa, a: &crate::luz::Mapa, dura: Duration) {
        if self.sin_fundir.get() {
            return;
        }
        let inicio = Instant::now();
        loop {
            let x = (inicio.elapsed().as_secs_f32() / dura.as_secs_f32()).min(1.0);
            if self.cuadro_mapa(&crate::luz::mezclar_mapa(de, a, x)).is_err() || x >= 1.0 {
                return;
            }
            sleep(PASO_FUNDIR);
        }
    }

    /// Mientras se espera (a que sueltes las teclas, a que conteste), lo que
    /// sostiene `chispa hold` se repite aquí: el de fondo no puede, el candado
    /// lo tiene este, y sin cuadros el teclado regresa a lo guardado.
    fn mantener(&self) {
        if self.mantenido.get().elapsed() < Duration::from_millis(300) {
            return;
        }
        self.mantenido.set(Instant::now());
        if crate::video::activo() {
            return;
        }
        if let Some(m) = crate::sostener::sostenido() {
            let _ = self.cuadro_mapa(&m);
        }
    }

    pub fn guardar_sin_fundir(&self) {
        self.sin_fundir.set(true);
    }

    pub fn ver_como(&self, m: Option<crate::luz::Mapa>) {
        *self.visto.borrow_mut() = m;
    }

    pub fn visto(&self) -> Option<crate::luz::Mapa> {
        self.visto.borrow().clone()
    }

    fn cuadro_radio(&self, grupos: &[crate::musica::Grupo]) -> Result<(), String> {
        for p in radio::cuadro(&crate::musica::a_bytes(grupos)) {
            self.mandar(&p)?;
            sleep(PAUSA_VIVO);
        }
        self.vaciar()
    }

    /// La batería en %, solo por radio (sin confirmar: ver radio::LEER_BATERIA).
    pub fn bateria(&self) -> Result<u8, String> {
        let b = self.leer(Cmd::LeerBateria, 2)?;
        b.first().copied().filter(|n| *n <= 100).ok_or_else(|| l("la batería contestó algo raro", "odd battery reply"))
    }

    pub fn enlace(&self) -> Enlace {
        self.enlace
    }

    /// Dejó de contestar: ya no hay que mandarle nada, ni para deshacer.
    pub fn colgado(&self) -> bool {
        self.colgado.get()
    }

    /// Pide un bloque y regresa sus `n` bytes de datos, revisando la cabecera.
    pub fn leer(&self, cmd: Cmd, n: usize) -> Result<Vec<u8>, String> {
        if self.enlace != Enlace::Cable {
            return self.leer_radio(cmd, n);
        }
        if cmd == Cmd::LeerBateria {
            return Err(l("por cable el teclado no dice su batería", "the keyboard doesn't report its battery over cable"));
        }
        let cab = cmd.cabecera();
        let mut buf = [0u8; LEN];
        buf[..HEADER].copy_from_slice(&cab);
        self.ioctl(SET_FEATURE, &mut buf)?;
        sleep(PAUSA_LEER);
        let mut r = [0u8; LEN];
        r[0] = REPORT;
        let got = self.ioctl(GET_FEATURE, &mut r)?;
        sleep(PAUSA_LEER);
        if got < HEADER + n || r[..6] != cab[..6] {
            return Err(format!("{} ({got} bytes, {:02x?})", l("respuesta rara del teclado", "unexpected keyboard reply"), &r[..HEADER]));
        }
        Ok(r[HEADER..HEADER + n].to_vec())
    }

    /// Escribe un bloque. Esto se guarda en la memoria del teclado.
    pub fn escribir(&self, cmd: Cmd, datos: &[u8]) -> Result<(), String> {
        if self.enlace != Enlace::Cable {
            return self.escribir_radio(cmd, datos);
        }
        if datos.len() > LEN - HEADER {
            return Err(l("bloque demasiado grande", "block too large"));
        }
        let mut buf = [0u8; LEN];
        buf[..HEADER].copy_from_slice(&cmd.cabecera());
        buf[HEADER..HEADER + datos.len()].copy_from_slice(datos);
        self.ioctl(SET_FEATURE, &mut buf)?;
        sleep(PAUSA_ESCRIBIR);
        Ok(())
    }

    fn ioctl(&self, req: u64, buf: &mut [u8; LEN]) -> Result<usize, String> {
        // SAFETY: buf mide exactamente LEN, el tamaño codificado en req.
        let r = unsafe { libc::ioctl(self.f.as_raw_fd(), req as _, buf.as_mut_ptr()) };
        if r < 0 {
            return Err(self.fallo(&std::io::Error::last_os_error()));
        }
        Ok(r as usize)
    }

    fn fallo(&self, e: &std::io::Error) -> String {
        if matches!(e.raw_os_error(), Some(libc::ETIMEDOUT | libc::ENODEV | libc::EIO | libc::EPIPE)) {
            self.colgado.set(true);
            return l(
                "el teclado no responde: no le mandé nada más. Revisa que siga conectado antes de volver a usar chispa",
                "the keyboard isn't responding: sent nothing else. Check it's still connected before using chispa again",
            ) + &format!(" ({e})");
        }
        format!("hidraw: {e}")
    }

    fn dormido(&self) -> String {
        self.colgado.set(true);
        l(
            "el teclado no contesta: puede estar dormido o en otro modo. Aprieta una tecla y vuelve a intentar",
            "the keyboard didn't answer: it may be asleep or in another mode. Press a key and try again",
        )
    }

    fn mandar(&self, p: &[u8; radio::LARGO]) -> Result<(), String> {
        if !radio::permitido(p) {
            return Err(l("ese paquete no sale de chispa", "chispa doesn't send that packet"));
        }
        let n = (&self.f).write(p).map_err(|e| self.fallo(&e))?;
        if n != p.len() {
            return Err(self.fallo(&std::io::Error::from_raw_os_error(libc::EIO)));
        }
        Ok(())
    }

    fn recibir(&self, espera: Duration) -> Result<Option<[u8; 64]>, String> {
        if !hay_datos(&self.f, espera).map_err(|e| self.fallo(&e))? {
            return Ok(None);
        }
        let mut b = [0u8; 64];
        (&self.f).read(&mut b).map_err(|e| self.fallo(&e))?;
        Ok(Some(b))
    }

    /// Tira lo que quedó en la cola: también llega el teclado normal por aquí.
    fn vaciar(&self) -> Result<(), String> {
        while self.recibir(Duration::ZERO)?.is_some() {}
        Ok(())
    }

    /// Espera a que no haya teclas abajo y lleven `quieto` sin moverse, para
    /// no perder la tecla que se está soltando (la compu la repetiría sin parar).
    fn esperar_teclas(&self, quieto: Duration) -> Result<(), String> {
        if self.teclas.is_empty() {
            sleep(quieto);
            return Ok(());
        }
        let inicio = Instant::now();
        let bt = self.enlace == Enlace::Bluetooth;
        loop {
            // antes de empezar, nunca entre paquetes: a media escritura la echa a perder
            if puede_mantener(quieto) {
                self.mantener();
            }
            let mut abajo = self.abajo.borrow_mut();
            let hay_abajo = abajo.iter().any(|a| *a);
            let espera = if quieto.is_zero() && !hay_abajo { Duration::ZERO } else { Duration::from_millis(50) };
            let mut fds: Vec<libc::pollfd> = self.teclas.iter().map(|f| libc::pollfd { fd: f.as_raw_fd(), events: libc::POLLIN, revents: 0 }).collect();
            // SAFETY: fds vive durante la llamada y su largo es el que se pasa.
            let r = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, espera.as_millis() as libc::c_int) };
            if r > 0 {
                for (i, p) in fds.iter().enumerate() {
                    if p.revents & libc::POLLIN == 0 {
                        continue;
                    }
                    let mut b = [0u8; 64];
                    if let Ok(n) = (&self.teclas[i]).read(&mut b)
                        && let Some(a) = radio::teclas_abajo(&b[..n], bt)
                    {
                        abajo[i] = a;
                        self.ultima_tecla.set(Instant::now());
                    }
                }
            }
            if !abajo.iter().any(|a| *a) && self.ultima_tecla.get().elapsed() >= quieto {
                return Ok(());
            }
            if inicio.elapsed() > MAX_TECLA_ABAJO {
                return Err(l("hay una tecla abajo: suéltalas todas y vuelve a intentar", "a key is held down: release them all and try again"));
            }
        }
    }

    fn leer_radio(&self, cmd: Cmd, n: usize) -> Result<Vec<u8>, String> {
        let (r, largo) = cmd.radio();
        let bt = self.enlace == Enlace::Bluetooth;
        if bt
            && let Some(falta) = self.ultima_escritura.get().and_then(|u| REPOSO_BT.checked_sub(u.elapsed()))
        {
            sleep(falta);
        }
        let mut armado = radio::Armado::default();
        for intento in 0..if bt { 15 } else { 4 } {
            // sin ninguna respuesta no se insiste: está dormido o en otro modo
            if intento >= 3 && armado.vacio() {
                break;
            }
            self.esperar_teclas(QUIETO)?;
            self.vaciar()?;
            if bt {
                sleep(al_azar());
            }
            self.mandar(&radio::pedir(r))?;
            let limite = Instant::now() + ESPERA_RAFAGA;
            let mut ultimo: Option<Instant> = None;
            while Instant::now() < limite && ultimo.is_none_or(|u| u.elapsed() <= FIN_RAFAGA) {
                let Some(p) = self.recibir(Duration::from_millis(50))? else { continue };
                let Some(rep) = radio::abrir(&p).filter(|x| x.cmd == r) else { continue };
                if rep.error {
                    return Err(format!("{} ({r:#04x})", l("el teclado rechazó la lectura", "the keyboard rejected the read")));
                }
                ultimo = Some(Instant::now());
                armado.agregar(&rep);
                if let Some(mut todo) = armado.completo() {
                    // la batería contesta 1 o 2 bytes
                    let bateria_ok = cmd == Cmd::LeerBateria && !todo.is_empty() && todo.len() <= largo;
                    if todo.len() != largo && !bateria_ok {
                        return Err(format!("{} ({} bytes)", l("respuesta rara del teclado", "unexpected keyboard reply"), todo.len()));
                    }
                    // la tabla por cable trae 6 bytes más; por radio no existen
                    todo.resize(n.max(largo), 0);
                    return Ok(todo);
                }
            }
        }
        if armado.vacio() {
            return Err(self.dormido());
        }
        Err(l("se perdieron paquetes de la respuesta; vuelve a intentar", "reply packets got lost; try again"))
    }

    fn escribir_radio(&self, cmd: Cmd, datos: &[u8]) -> Result<(), String> {
        let (c, largo) = cmd.radio();
        // la tabla por cable trae 6 bytes más; por radio no existen
        let datos = datos.get(..largo).ok_or_else(|| l("bloque demasiado corto", "block too short"))?;
        self.esperar_teclas(QUIETO)?;
        self.vaciar()?;
        for p in radio::partir(c, datos) {
            self.esperar_teclas(Duration::ZERO)?;
            let mut eco = false;
            for _ in 0..3 {
                self.mandar(&p)?;
                if self.esperar_eco(&p)? {
                    eco = true;
                    break;
                }
            }
            if !eco {
                return Err(self.dormido());
            }
            // nada en vivo entre paquetes: probado, el teclado guarda mal
            sleep(PAUSA_PAQUETE);
        }
        self.ultima_escritura.set(Some(Instant::now()));
        sleep(PAUSA_ESCRIBIR);
        Ok(())
    }

    /// El teclado regresa cada paquete que recibe.
    fn esperar_eco(&self, p: &[u8; radio::LARGO]) -> Result<bool, String> {
        let limite = Instant::now() + ESPERA_ECO;
        while Instant::now() < limite {
            let Some(b) = self.recibir(Duration::from_millis(100))? else { continue };
            let Some(rep) = radio::abrir(&b).filter(|x| x.cmd == p[1] && x.indice == p[3] as usize) else { continue };
            if rep.error {
                return Err(format!("{} ({:#04x})", l("el teclado rechazó el paquete", "the keyboard rejected the packet"), p[1]));
            }
            return Ok(true);
        }
        Ok(false)
    }
}

/// Todos los nodos del mismo teclado, solo para leer. Cada uno tiene su propia
/// copia de los reportes: leerlos aquí no le quita nada a la compu.
fn nodos_de_teclas(id: &str) -> Vec<File> {
    let Ok(dir) = fs::read_dir("/sys/class/hidraw") else { return Vec::new() };
    dir.flatten()
        .filter(|e| uevent(&e.path(), "HID_ID").as_deref() == Some(id))
        .filter_map(|e| {
            OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(PathBuf::from("/dev").join(e.file_name()))
                .ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {

    #[test]
    fn por_cable_nunca_hay_cuadros_en_vivo() {
        // se comía las teclas: 66 ms por cuadro sin leer el teclado
        assert!(!Enlace::Cable.en_vivo());
        assert!(Enlace::Dongle.en_vivo() && Enlace::Bluetooth.en_vivo());
    }

    #[test]
    fn nunca_un_cuadro_a_media_escritura() {
        // escribir_radio espera con QUIETO antes y con 0 entre paquetes
        assert!(puede_mantener(QUIETO));
        assert!(!puede_mantener(Duration::ZERO));
    }

    use super::*;

    const DESC_DONGLE: &[u8] = &[0x06, 0x02, 0xff, 0x09, 0x02, 0xa1, 0x01, 0x85, 0x13, 0x15, 0x00];

    #[test]
    fn por_radio_nunca_sale_el_reset_de_fabrica() {
        // por cable 0x06 escribe la tabla; por radio borra todo
        for c in [Cmd::LeerLuz, Cmd::EscribirLuz, Cmd::LeerTabla, Cmd::EscribirTabla] {
            let (n, _) = c.radio();
            assert_ne!(n, 0x06);
            assert!(radio::permitido(&radio::pedir(n)));
        }
        assert_eq!(Cmd::EscribirTabla.cabecera()[1], 0x06, "por cable sí es la tabla");
        assert_eq!(Cmd::EscribirTabla.radio().0, 0x02);
    }

    #[test]
    fn cada_nodo_es_de_su_enlace() {
        assert_eq!(enlace_de("0003:00003554:0000FA09", DESC_DONGLE), Some(Enlace::Dongle));
        assert_eq!(enlace_de("0005:00003554:0000FA07", DESC_DONGLE), Some(Enlace::Bluetooth));
        assert_eq!(enlace_de("0005:00003554:0000FA08", DESC_DONGLE), Some(Enlace::Bluetooth));
        assert_eq!(enlace_de("0003:00003554:0000FA09", &[0x05, 0x01]), None, "la interfaz del teclado normal no");
        assert_eq!(enlace_de("0003:0000046D:0000C54D", DESC_DONGLE), None, "otro aparato no");
        assert_eq!(enlace_de("0003:0000258A:0000010C", &[0x85, 0x06, 0x96, 0x07, 0x02]), Some(Enlace::Cable));
        assert_eq!(enlace_de("0003:0000258A:0000010C", DESC_DONGLE), None);
    }
}
