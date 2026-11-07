//! chispa: la luz del AULA F75 desde la terminal. Lo que se pone se queda
//! guardado en el teclado; los cambios de color de todo el teclado se ven en
//! vivo y se guardan después (`chispa hold`, ver sostener.rs), porque guardar
//! siempre parpadea.

mod hid;
mod luz;
mod musica;
mod radio;
mod sostener;
mod tui;
mod video;

use hid::{Cmd, Teclado};
use luz::{LARGO_LUZ, LARGO_TABLA, MULTI};
use ratatui::style::Color;
use std::path::PathBuf;

fn es() -> bool {
    let f = PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/hypr/discada.conf");
    std::fs::read_to_string(f).is_ok_and(|t| t.lines().any(|l| l.starts_with("LANGUAGE=es")))
}

pub(crate) fn l(es_: &str, en: &str) -> String {
    if es() { es_.into() } else { en.into() }
}

const AYUDA_ES: &str = "chispa: la luz del AULA F75 (por cable, 2.4 GHz o Bluetooth)

  chispa                     cómo está la luz
  chispa modes               los efectos
  chispa mode <efecto> [--brightness 0-9] [--speed 0-4] [--color <color|multi>]
  chispa brightness <0-9>    brillo del efecto actual
  chispa speed <0-4>         velocidad del efecto actual
  chispa color <color>       todo el teclado de un color (#b9b6d8, red…)
  chispa key <tecla> <color> [<tecla> <color>…]   tecla por tecla
  chispa keys                los nombres de las teclas
  chispa look                el teclado con el color del look de discada
  chispa wallpaper           el teclado sigue a los colores del wallpaper
  chispa music               el visualizador: barras con el audio (Ctrl+C o `music stop`)
  chispa video               con wallpaper en video, el teclado lo sigue en vivo (solo, al seguir al wallpaper)
  chispa off | on            apagar, y regresar al último efecto
  chispa side <modo>         la barrita junto al Esc: off fill breathing static
  chispa sleep [never|<min>] apagar las luces sin usarlo (2.4 GHz/Bluetooth), 0.5–20 min
  chispa battery [--cached]  la batería (solo por 2.4 GHz); --cached: la última, sin preguntarle
  chispa save                guardar ya el color en vivo (si no, al bloquear la pantalla)
  chispa restore             regresar a como estaba antes de usar chispa
  chispa tui                 todo esto en una ventana, con el teclado dibujado

Colores: red orange yellow green cyan blue purple pink white, off, o #rrggbb.
Los de `mode` solo: red green blue yellow purple cyan white multi.";

const AYUDA_EN: &str = "chispa: lighting for the AULA F75 (wired, 2.4 GHz or Bluetooth)

  chispa                     show the current lighting
  chispa modes               list the effects
  chispa mode <effect> [--brightness 0-9] [--speed 0-4] [--color <color|multi>]
  chispa brightness <0-9>    brightness of the current effect
  chispa speed <0-4>         speed of the current effect
  chispa color <color>       whole keyboard one color (#b9b6d8, red…)
  chispa key <key> <color> [<key> <color>…]   key by key
  chispa keys                list the key names
  chispa look                paint the keyboard with the discada look's color
  chispa wallpaper           the keyboard follows the wallpaper's colors
  chispa music               the visualizer: bars with the audio (Ctrl+C or `music stop`)
  chispa video               with a video wallpaper, the keyboard follows it live (automatic when following the wallpaper)
  chispa off | on            turn off, and back to the last effect
  chispa side <mode>         the light bar next to Esc: off fill breathing static
  chispa sleep [never|<min>] lights off when idle (2.4 GHz/Bluetooth), 0.5–20 min
  chispa battery [--cached]  battery level (2.4 GHz only); --cached: the last one, without asking
  chispa save                save the live color now (otherwise on screen lock)
  chispa restore             back to how it was before using chispa
  chispa tui                 all of this in a window, with the keyboard drawn

Colors: red orange yellow green cyan blue purple pink white, off, or #rrggbb.
`mode` only takes: red green blue yellow purple cyan white multi.";

/// Qué fundido toca antes de guardar (de, a), con lo que se ve sin negros: al
/// apagar, a negro; al prender, desde negro (apagado sí enseña cuadros, probado);
/// a otros colores, del de antes al nuevo. Nada si no hay qué fundir.
fn fundido(antes: u8, ahora: u8, de: Option<luz::Mapa>, a: Option<luz::Mapa>) -> Option<(luz::Mapa, luz::Mapa)> {
    let negro = luz::mapa_uniforme((0, 0, 0));
    match (antes != 0, de, a) {
        (true, Some(de), _) if ahora == 0 => Some((de, negro)),
        (true, Some(de), Some(a)) if de != a => Some((de, a)),
        (false, _, Some(a)) => Some((negro, a)),
        _ => None,
    }
}

/// lo que tarda en pasar de un color a otro
const FUNDIDO: std::time::Duration = std::time::Duration::from_millis(400);

/// unos 2 minutos de intentos
const REINTENTOS_WALLPAPER: u32 = 40;
const PAUSA_WALLPAPER: std::time::Duration = std::time::Duration::from_secs(3);

fn discada_conf() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/hypr/discada.conf")
}

pub(crate) fn modo_actual() -> Option<String> {
    std::fs::read_to_string(discada_conf()).ok().and_then(|t| luz::modo_de(&t))
}

pub(crate) fn sigue_wallpaper() -> bool {
    modo_actual().as_deref() == Some("wallpaper")
}

pub(crate) fn en_musica() -> bool {
    modo_actual().as_deref() == Some("music")
}

/// Recuerda si el teclado sigue al wallpaper. Solo se escribe si cambió.
pub(crate) fn poner_modo(modo: &str) -> Result<(), String> {
    let f = discada_conf();
    let t = std::fs::read_to_string(&f).map_err(|e| format!("{}: {e}", f.display()))?;
    if luz::modo_de(&t).as_deref() == Some(modo) {
        return Ok(());
    }
    std::fs::write(&f, luz::con_modo(&t, modo)).map_err(|e| format!("{}: {e}", f.display()))
}

/// Lo que se escoge a mano ya no lo pisa el siguiente wallpaper, y apaga el visualizador.
fn a_mano() {
    if (sigue_wallpaper() || en_musica())
        && let Err(e) = poner_modo("custom")
    {
        eprintln!("chispa: {e}");
    }
}

/// Instalaron otro chispa encima del que corre: los de fondo (hold, video) se
/// reinician solos con el nuevo, o seguirían con el código viejo (pasó: un hold
/// viejo pintaba azul encima del video).
pub(crate) fn binario_viejo() -> bool {
    std::fs::read_link("/proc/self/exe").is_ok_and(|p| p.to_string_lossy().ends_with(" (deleted)"))
}

/// Se vuelve a lanzar con el binario nuevo, mismo pid y mismos argumentos.
pub(crate) fn reiniciar() {
    use std::os::unix::process::CommandExt;
    anotar("binario nuevo: me reinicio con él");
    let Ok(p) = std::fs::read_link("/proc/self/exe") else { return };
    let ruta = PathBuf::from(p.to_string_lossy().trim_end_matches(" (deleted)").to_string());
    let e = std::process::Command::new(ruta).args(std::env::args_os().skip(1)).exec();
    eprintln!("chispa: {e}");
}

/// Si el programa está en el PATH (sin correrlo).
pub(crate) fn hay_programa(nombre: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(nombre).is_file()))
}

fn estado_dir() -> PathBuf {
    let base = std::env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/state"));
    base.join("chispa")
}

pub(crate) fn respaldo() -> PathBuf {
    estado_dir().join("respaldo.bin")
}

pub(crate) fn leer_luz(t: &Teclado) -> Result<Vec<u8>, String> {
    let b = t.leer(Cmd::LeerLuz, LARGO_LUZ)?;
    if !luz::valido(&b) {
        return Err(l("el bloque de ajustes no trae su firma; no escribí nada", "the settings block has no signature; nothing was written"));
    }
    Ok(b)
}

pub(crate) fn leer_tabla(t: &Teclado) -> Result<Vec<u8>, String> {
    t.leer(Cmd::LeerTabla, LARGO_TABLA)
}

/// Lo primero que se escribe en un teclado se respalda antes, y nunca se pisa.
fn respaldar(luz_: &[u8], tabla: &[u8]) -> Result<(), String> {
    let f = respaldo();
    if f.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(estado_dir()).map_err(|e| format!("{}: {e}", estado_dir().display()))?;
    let mut todo = luz_.to_vec();
    todo.extend_from_slice(tabla);
    std::fs::write(&f, todo).map_err(|e| format!("{}: {e}", f.display()))
}

/// Escribe solo lo que cambió, lo vuelve a leer, y si no quedó igual regresa
/// el teclado a como estaba.
pub(crate) fn guardar(t: &Teclado, antes: &[u8], ahora: &[u8], tabla: Option<(&[u8], &[u8])>) -> Result<bool, String> {
    if !luz::solo_luz(antes, ahora) {
        return Err(l("iba a cambiar algo que no es de la luz; no escribí nada", "that would change something other than lighting; nothing was written"));
    }
    let cambia_tabla = tabla.is_some_and(|(a, b)| !luz::misma_tabla(a, b));
    if antes == ahora && !cambia_tabla {
        return Ok(false);
    }
    match tabla {
        Some((a, _)) => respaldar(antes, a)?,
        None => respaldar(antes, &leer_tabla(t)?)?,
    }
    // Lo que se ve antes y después, si es un solo color. Por radio, todo lo que
    // se guarda apaga las luces un rato, y después enseña lo guardado de golpe e
    // ignora los cuadros en vivo (probado: cuadros a media escritura la echan a
    // perder, 1.5 s de cuadros negros después no alcanzan, la paleta del
    // estático igual, y por cable también parpadea aunque se sostenga en vivo). Así que solo se funde ANTES de guardar: al apagar, a
    // negro; a otro color, del de antes al nuevo; al prender, desde negro.
    let (de, a) = tabla.map_or((None, None), |(vieja, nueva)| (luz::visible_mapa(antes, vieja), luz::visible_mapa(ahora, nueva)));
    // lo sostenido en vivo es lo que se ve, aunque lo guardado sea otro
    let de = t.visto().or(de).filter(|m| !luz::es_negro(m));
    let a = a.filter(|m| !luz::es_negro(m));
    if let Some((de, a)) = fundido(luz::activo(antes), luz::activo(ahora), de, a) {
        t.fundir(&de, &a, FUNDIDO);
    }
    let intento = (|| -> Result<(), String> {
        // primero se prende la tabla, luego sus colores
        if antes != ahora {
            t.escribir(Cmd::EscribirLuz, ahora)?;
        }
        let escribio_tabla = tabla.filter(|_| cambia_tabla).map(|(_, nueva)| t.escribir(Cmd::EscribirTabla, nueva)).transpose()?.is_some();
        if escribio_tabla
            && let Some((_, nueva)) = tabla
            && !luz::misma_tabla(&leer_tabla(t)?, nueva)
        {
            return Err(l("la tabla de colores no quedó igual", "the color table did not read back the same"));
        }
        if t.leer(Cmd::LeerLuz, LARGO_LUZ)? != ahora {
            return Err(l("los ajustes no quedaron igual", "the settings did not read back the same"));
        }
        Ok(())
    })();
    if let Err(e) = intento {
        if t.colgado() {
            return Err(e);
        }
        let mut msg = format!("{e}; {}", l("regresé el teclado a como estaba", "put the keyboard back how it was"));
        if let Some((vieja, _)) = tabla.filter(|_| cambia_tabla)
            && let Err(e2) = t.escribir(Cmd::EscribirTabla, vieja)
        {
                msg = format!("{e}; {}: {e2}", l("y no pude regresar la tabla", "and could not restore the table"));
        }
        if let Err(e2) = t.escribir(Cmd::EscribirLuz, antes) {
            msg = format!("{e}; {}: {e2}", l("y no pude regresar los ajustes", "and could not restore the settings"));
        }
        return Err(msg);
    }
    Ok(true)
}

fn listo(cambio: bool, que: &str) {
    if cambio {
        println!("{que}");
    } else {
        println!("{que} ({})", l("ya estaba así, no escribí nada", "already set, nothing written"));
    }
}

fn nombre_color(c: u8) -> String {
    if c == MULTI {
        return "multi".into();
    }
    luz::COLORES.get(c as usize).map_or_else(|| c.to_string(), |x| x.0.to_string())
}

fn estado(t: &Teclado) -> Result<(), String> {
    let b = leer_luz(t)?;
    println!("{}: {}", l("enlace", "link"), t.enlace().nombre());
    if let Some(b) = texto_bateria(t) {
        println!("{}: {b}", l("batería", "battery"));
    }
    if let Some(m) = sostener::sostenido() {
        let que = match luz::grupos(&m, false).as_slice() {
            [((r, g, bl), _)] => format!("#{r:02x}{g:02x}{bl:02x}"),
            g => format!("{} {}", g.len(), l("colores", "colors")),
        };
        println!("{}: {que} ({})", l("en vivo", "live"), l("se guarda al bloquear la pantalla", "saved when the screen locks"));
    }
    println!("{}: {}", l("barrita", "side light"), nombre_lateral(luz::lateral(&b)));
    println!("{}: {}", l("apagar luces sin usarlo", "lights off when idle"), nombre_dormir(luz::dormir(&b)));
    let id = luz::activo(&b);
    if id == 0 {
        println!("{}", l("apagado", "off"));
        return Ok(());
    }
    let (br, sp, co) = luz::ajustes(&b, id);
    let e = luz::efecto_id(id);
    println!("{}: {}", l("efecto", "effect"), e.map_or_else(|| id.to_string(), |e| e.nombre.to_string()));
    println!("{}: {br}/9", l("brillo", "brightness"));
    if e.is_none_or(|e| e.velocidad) {
        println!("{}: {sp}/4", l("velocidad", "speed"));
    }
    if e.is_none_or(|e| e.color) {
        println!("color: {}", nombre_color(co));
    }
    Ok(())
}

pub(crate) fn nombre_dormir(m: u8) -> String {
    luz::minutos(m).unwrap_or_else(|| l("nunca", "never"))
}

/// "never" o minutos (de 0.5 en 0.5, hasta 20), a medios minutos
fn medios_minutos(s: &str) -> Result<u8, String> {
    if matches!(s, "never" | "nunca" | "off" | "0") {
        return Ok(0);
    }
    let m: f32 = s.trim_end_matches("min").trim().parse().map_err(|_| l("sleep: never o minutos (1, 1.5 … 20)", "sleep: never or minutes (1, 1.5 … 20)"))?;
    let medios = (m * 2.0).round();
    if !(1.0..=luz::MAX_DORMIR as f32).contains(&medios) {
        return Err(l("sleep: de 0.5 a 20 minutos, o never", "sleep: 0.5 to 20 minutes, or never"));
    }
    Ok(medios as u8)
}

fn sleep_(t: &Teclado, medios: Option<u8>) -> Result<(), String> {
    let antes = leer_luz(t)?;
    let Some(m) = medios else {
        println!("{}", nombre_dormir(luz::dormir(&antes)));
        return Ok(());
    };
    let mut b = antes.clone();
    luz::poner_dormir(&mut b, m);
    listo(guardar(t, &antes, &b, None)?, &format!("{}: {}", l("dormir", "sleep"), nombre_dormir(m)));
    Ok(())
}

/// Una vez que se ve abajo de 100, ya se sabe que sí es la batería: se anota.
fn bateria_confirmada() -> bool {
    estado_dir().join("bateria-confirmada").exists()
}

/// La última batería que se leyó: "n segundos", para enseñarla sin hablarle al
/// teclado (por radio, con el video mandando cuadros, leer tarda ~5 s).
fn bateria_guardada() -> PathBuf {
    estado_dir().join("bateria")
}

fn anotar_bateria(n: u8) {
    let ahora = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let _ = std::fs::create_dir_all(estado_dir());
    let _ = std::fs::write(bateria_guardada(), format!("{n} {ahora}\n"));
}

/// "87%" o "100% (?)", y hace cuánto se leyó, sin tocar el teclado.
fn bateria_de_memoria() -> Option<(String, u64)> {
    let t = std::fs::read_to_string(bateria_guardada()).ok()?;
    let (n, cuando) = t.trim().split_once(' ')?;
    let (n, cuando): (u8, u64) = (n.parse().ok()?, cuando.parse().ok()?);
    let ahora = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let texto = if bateria_confirmada() || n < 100 { format!("{n}%") } else { format!("{n}% (?)") };
    Some((texto, ahora.saturating_sub(cuando)))
}

pub(crate) fn texto_bateria(t: &Teclado) -> Option<String> {
    let n = t.bateria().ok()?;
    anotar_bateria(n);
    if n < 100 && !bateria_confirmada() {
        let _ = std::fs::create_dir_all(estado_dir());
        let _ = std::fs::write(estado_dir().join("bateria-confirmada"), n.to_string());
    }
    Some(if bateria_confirmada() { format!("{n}%") } else { format!("{n}% (?)") })
}

fn battery(t: &Teclado) -> Result<(), String> {
    let n = t.bateria()?;
    anotar_bateria(n);
    let nota = if bateria_confirmada() || n < 100 {
        String::new()
    } else {
        format!(" ({})", l("sin confirmar: falta verla bajar de 100", "unconfirmed: not yet seen below 100"))
    };
    println!("{n}%{nota}");
    Ok(())
}

fn nombre_lateral(m: u8) -> String {
    luz::MODOS_LATERAL.get(m as usize).map_or_else(|| m.to_string(), |x| x.to_string())
}

fn modo_lateral(s: &str) -> Result<u8, String> {
    luz::MODOS_LATERAL.iter().position(|m| *m == s).map(|i| i as u8).ok_or_else(|| {
        format!("{}: off fill breathing static", l("la barrita solo tiene", "the side light only has"))
    })
}

fn side(t: &Teclado, modo: u8) -> Result<(), String> {
    let antes = leer_luz(t)?;
    let mut b = antes.clone();
    luz::poner_lateral(&mut b, modo);
    listo(guardar(t, &antes, &b, None)?, &format!("{}: {}", l("barrita", "side light"), nombre_lateral(modo)));
    Ok(())
}

fn numero(s: &str, max: u8, que: &str) -> Result<u8, String> {
    s.parse::<u8>().ok().filter(|n| *n <= max).ok_or_else(|| format!("{que}: 0-{max}"))
}

fn color_de_efecto(s: &str) -> Result<u8, String> {
    if s == "multi" {
        return Ok(MULTI);
    }
    luz::COLORES.iter().position(|c| c.0 == s).map(|i| i as u8).ok_or_else(|| {
        l(
            "ese color no: los efectos solo traen red green blue yellow purple cyan white multi",
            "not that color: effects only have red green blue yellow purple cyan white multi",
        )
    })
}

fn mode(t: &Teclado, args: &[String]) -> Result<(), String> {
    let nombre = args.first().ok_or_else(|| l("¿cuál efecto? (chispa modes)", "which effect? (chispa modes)"))?;
    let e = luz::efecto(nombre).ok_or_else(|| format!("{}: {nombre} (chispa modes)", l("no conozco ese efecto", "unknown effect")))?;
    let (mut br, mut sp, mut co) = (None, None, None);
    let mut it = args[1..].iter();
    while let Some(k) = it.next() {
        let v = it.next().ok_or_else(|| format!("{k}: {}", l("falta el valor", "missing value")))?;
        match k.as_str() {
            "--brightness" => br = Some(numero(v, 9, "brightness")?),
            "--speed" => sp = Some(numero(v, 4, "speed")?),
            "--color" => co = Some(color_de_efecto(v)?),
            _ => return Err(format!("{}: {k}", l("no conozco esa opción", "unknown option"))),
        }
    }
    if sp.is_some() && !e.velocidad {
        return Err(format!("{}: {nombre}", l("este efecto no tiene velocidad", "this effect has no speed")));
    }
    if co.is_some() && !e.color {
        return Err(format!("{}: {nombre}", l("este efecto no tiene color", "this effect has no color")));
    }
    let antes = leer_luz(t)?;
    let mut b = antes.clone();
    luz::poner(&mut b, e.id, br, sp, co);
    listo(guardar(t, &antes, &b, None)?, e.nombre);
    a_mano();
    Ok(())
}

fn ajustar(t: &Teclado, br: Option<u8>, sp: Option<u8>) -> Result<(), String> {
    let antes = leer_luz(t)?;
    let id = luz::activo(&antes);
    if id == 0 {
        return Err(l("está apagado (chispa on)", "it's off (chispa on)"));
    }
    if sp.is_some() && luz::efecto_id(id).is_some_and(|e| !e.velocidad) {
        return Err(l("este efecto no tiene velocidad", "this effect has no speed"));
    }
    let mut b = antes.clone();
    luz::poner(&mut b, id, br, sp, None);
    let que = br.map_or_else(|| format!("{}/4", sp.unwrap_or(0)), |x| format!("{x}/9"));
    listo(guardar(t, &antes, &b, None)?, &que);
    Ok(())
}

/// El teclado con este mapa sin parpadear: se funde en vivo desde lo que se ve,
/// y el de fondo lo sostiene hasta guardarlo cuando nadie lo vea.
pub(crate) fn cambiar_mapa(t: &Teclado, m: &luz::Mapa) -> Result<bool, String> {
    // por cable no hay en vivo (se come las teclas): se guarda luego luego, con
    // su parpadeo. Lo que sostenía el de fondo queda reemplazado por esto.
    if !t.en_vivo() {
        // el de fondo pudo quedarse esperando el candado: se suelta mientras se va
        t.soltar();
        sostener::terminar(false);
        t.tomar();
        let teclas: Vec<(usize, luz::Rgb)> = m.iter().map(|(led, c)| (*led as usize, *c)).collect();
        return aplicar_tabla(t, &teclas);
    }
    let de = match sostener::sostenido() {
        Some(h) => Some(h),
        None => {
            let b = leer_luz(t)?;
            luz::visible_mapa(&b, &leer_tabla(t)?)
        }
    };
    if de.as_ref() == Some(m) {
        return Ok(false);
    }
    t.fundir(de.as_ref().unwrap_or(m), m, FUNDIDO);
    sostener::sostener(m)?;
    Ok(true)
}

pub(crate) fn cambiar_color(t: &Teclado, c: luz::Rgb) -> Result<bool, String> {
    cambiar_mapa(t, &luz::mapa_uniforme(c))
}

/// Pinta teclas en la tabla y deja prendido el efecto de la tabla.
pub(crate) fn aplicar_tabla(t: &Teclado, teclas: &[(usize, (u8, u8, u8))]) -> Result<bool, String> {
    let antes = leer_luz(t)?;
    let tabla = leer_tabla(t)?;
    let mut nueva = tabla.clone();
    for (led, c) in teclas {
        luz::pintar(&mut nueva, *led, *c);
    }
    let mut b = antes.clone();
    luz::poner(&mut b, luz::PERSONAL, None, None, None);
    guardar(t, &antes, &b, Some((&tabla, &nueva)))
}

fn pintar(t: &Teclado, teclas: &[(usize, (u8, u8, u8))], que: &str) -> Result<(), String> {
    listo(aplicar_tabla(t, teclas)?, que);
    a_mano();
    Ok(())
}

/// El wallpaper de ahora: el último que le pasaron a WallustSwww.sh.
fn wallpaper_actual() -> Result<PathBuf, String> {
    let link = PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/hypr/wallpaper_effects/.wallpaper_link");
    std::fs::canonicalize(&link).map_err(|e| format!("{}: {e}", link.display()))
}

/// El color del wallpaper. wallust solo imprime su paleta: no toca plantillas,
/// ni la terminal, ni los colores del look.
pub(crate) fn color_del_wallpaper(img: Option<&str>) -> Result<(u8, u8, u8), String> {
    let img = match img {
        Some(i) => PathBuf::from(i),
        None => wallpaper_actual()?,
    };
    let out = std::process::Command::new("wallust")
        .args(["run", "-s", "-T", "-q", "--no-hooks", "--print-scheme"])
        .arg(&img)
        .output()
        .map_err(|e| format!("wallust: {e}"))?;
    let paleta: Vec<_> = String::from_utf8_lossy(&out.stdout).lines().filter_map(|c| luz::color(c.trim())).collect();
    luz::color_de_paleta(&paleta).ok_or_else(|| {
        format!(
            "{}: {}",
            l("no pude sacar colores de este wallpaper", "couldn't get colors from this wallpaper"),
            String::from_utf8_lossy(&out.stderr).trim()
        )
    })
}


/// Lo lanza WallustSwww.sh en cada wallpaper, y va en el fondo: funde al
/// wallpaper nuevo tecla por tecla y lo deja sostenido (ver sostener.rs). Si el
/// teclado no contesta (dormido, o una tecla abajo) se sigue intentando un rato,
/// hasta que le gane un wallpaper más nuevo.
fn seguir_wallpaper(img: Option<&str>) -> Result<(), String> {
    if !sigue_wallpaper() {
        return Ok(());
    }
    // wallpaper en video: el teclado lo sigue en vivo (y el color de abajo
    // queda sostenido para cuando se quite)
    if video::hay_mpvpaper() {
        video::lanzar_fondo();
    }
    let m = mapa_del_wallpaper(img)?;
    let vigente = || img.is_none_or(|i| std::fs::canonicalize(i).ok() == wallpaper_actual().ok());
    for _ in 0..REINTENTOS_WALLPAPER {
        if !sigue_wallpaper() || !vigente() {
            return Ok(());
        }
        // sin teclado conectado no hay a quién esperarle
        let Ok(t) = Teclado::abrir() else { return Ok(()) };
        // si ya se sostiene algo está prendido: no hace falta preguntarle (leer
        // tarda, y mientras el de fondo no puede repetir lo suyo)
        let r = if sostener::sostenido().is_some() {
            cambiar_mapa(&t, &m).map(|_| ())
        } else {
            leer_luz(&t).and_then(|b| {
                if luz::activo(&b) != 0 {
                    cambiar_mapa(&t, &m)?;
                }
                Ok(())
            })
        };
        if r.is_ok() {
            return Ok(());
        }
        drop(t);
        std::thread::sleep(PAUSA_WALLPAPER);
    }
    Err(l("no pude guardar el color del wallpaper", "couldn't save the wallpaper color"))
}

/// El wallpaper tecla por tecla: cada una con el pedazo de imagen que le queda
/// encima, como con los videos. Si la imagen no se deja leer, todo de un color.
pub(crate) fn mapa_del_wallpaper(img: Option<&str>) -> Result<luz::Mapa, String> {
    let ruta = match img {
        Some(i) => PathBuf::from(i),
        None => wallpaper_actual()?,
    };
    match video::mapa_de_imagen(&ruta) {
        Some(m) => Ok(m),
        None => color_del_wallpaper(img).map(luz::mapa_uniforme),
    }
}

fn wallpaper(args: &[String]) -> Result<(), String> {
    let auto = args.first().is_some_and(|a| a == "--auto");
    let img = args.get(usize::from(auto)).map(String::as_str);
    if auto {
        return seguir_wallpaper(img);
    }
    let m = mapa_del_wallpaper(img)?;
    let t = Teclado::abrir()?;
    listo(cambiar_mapa(&t, &m)?, &l("el wallpaper, tecla por tecla", "the wallpaper, key by key"));
    poner_modo("wallpaper")
}

fn color_o_error(s: &str) -> Result<(u8, u8, u8), String> {
    luz::color(s).ok_or_else(|| format!("{}: {s}", l("no entiendo ese color", "unknown color")))
}


fn key(t: &Teclado, args: &[String]) -> Result<(), String> {
    if args.is_empty() || !args.len().is_multiple_of(2) {
        return Err(l("chispa key <tecla> <color> [<tecla> <color>…]", "chispa key <key> <color> [<key> <color>…]"));
    }
    let mut teclas = Vec::new();
    for par in args.chunks(2) {
        let nombre = par[0].to_lowercase();
        let led = luz::TECLAS.iter().find(|k| k.0 == nombre).map(|k| k.1)
            .ok_or_else(|| format!("{}: {nombre} (chispa keys)", l("no conozco esa tecla", "unknown key")))?;
        teclas.push((led, color_o_error(&par[1])?));
    }
    pintar(t, &teclas, &args.join(" "))
}

pub(crate) fn color_del_look() -> Result<(u8, u8, u8), String> {
    let Color::Rgb(r, g, b) = disco::look::Look::load("chispa").theme.accent else {
        return Err(l("el look no trae un color exacto", "the look has no exact color"));
    };
    Ok(luz::oscurecer((r, g, b)))
}

fn look(t: &Teclado) -> Result<(), String> {
    let (r, g, b) = color_del_look()?;
    un_color(t, (r, g, b), &format!("#{r:02x}{g:02x}{b:02x}"))
}

/// Todo el teclado de un color a mano: sin parpadeo, y deja de seguir al wallpaper.
fn un_color(t: &Teclado, c: luz::Rgb, que: &str) -> Result<(), String> {
    listo(cambiar_color(t, c)?, que);
    a_mano();
    Ok(())
}

pub(crate) fn anotar_ultimo(id: u8) -> Result<(), String> {
    std::fs::create_dir_all(estado_dir()).map_err(|e| e.to_string())?;
    std::fs::write(estado_dir().join("ultimo"), id.to_string()).map_err(|e| e.to_string())
}

pub(crate) fn ultimo() -> Option<u8> {
    std::fs::read_to_string(estado_dir().join("ultimo"))
        .ok()
        .and_then(|s| s.trim().parse::<u8>().ok())
        .filter(|i| luz::efecto_id(*i).is_some())
}

/// `sostenido`: el color que tenía en vivo el de fondo (ya se fue sin guardar).
/// Se guarda junto con el apagado, a oscuras: no se ve.
fn off(t: &Teclado, sostenido: Option<luz::Mapa>) -> Result<(), String> {
    if en_musica() {
        poner_modo("custom")?;
    }
    let antes = leer_luz(t)?;
    let mut id = luz::activo(&antes);
    let tabla = leer_tabla(t)?;
    let mut nueva = tabla.clone();
    if let Some(m) = sostenido {
        for (led, c) in &m {
            luz::pintar(&mut nueva, *led as usize, *c);
        }
        id = luz::PERSONAL;
        t.ver_como(Some(m));
    }
    if id != 0 {
        anotar_ultimo(id)?;
    }
    let mut b = antes.clone();
    luz::apagar(&mut b);
    listo(guardar(t, &antes, &b, Some((&tabla, &nueva)))?, &l("apagado", "off"));
    Ok(())
}

fn on(t: &Teclado) -> Result<(), String> {
    let antes = leer_luz(t)?;
    if luz::activo(&antes) != 0 {
        println!("{}", l("ya está prendido", "already on"));
        return Ok(());
    }
    let id = ultimo().ok_or_else(|| l("no sé cuál estaba; escoge uno (chispa modes)", "don't know which was on; pick one (chispa modes)"))?;
    let mut b = antes.clone();
    luz::poner(&mut b, id, None, None, None);
    let tabla = leer_tabla(t)?;
    listo(guardar(t, &antes, &b, Some((&tabla, &tabla)))?, luz::efecto_id(id).map_or("", |e| e.nombre));
    Ok(())
}

/// Regresa los dos bloques al respaldo: la luz, la barrita y el sleep. Lo demás
/// del bloque de ajustes (Mac, polling, la perilla) se queda como está hoy.
pub(crate) fn restaurar(t: &Teclado) -> Result<bool, String> {
    let f = respaldo();
    let todo = std::fs::read(&f).map_err(|_| l("no hay respaldo: chispa todavía no ha escrito nada", "no backup: chispa hasn't written anything yet"))?;
    if todo.len() != LARGO_LUZ + LARGO_TABLA || !luz::valido(&todo[..LARGO_LUZ]) {
        return Err(format!("{}: {}", l("respaldo dañado", "damaged backup"), f.display()));
    }
    let (r_luz, r_tabla) = todo.split_at(LARGO_LUZ);
    let antes = leer_luz(t)?;
    let tabla = leer_tabla(t)?;
    // primero la tabla y la tabla de efectos: cada byte de efecto se regresa con un
    // poner por efecto, para que solo_luz pueda revisar cada escritura
    let mut actual = antes.clone();
    let mut cambio = false;
    for e in luz::EFECTOS {
        let (br, sp, co) = luz::ajustes(r_luz, e.id);
        if luz::ajustes(&actual, e.id) != (br, sp, co) {
            let mut b = actual.clone();
            luz::poner(&mut b, e.id, Some(br), Some(sp), Some(co));
            cambio |= guardar(t, &actual, &b, None)?;
            actual = b;
        }
    }
    let id = luz::activo(r_luz);
    let mut b = actual.clone();
    if id == 0 {
        luz::apagar(&mut b);
    } else {
        b[9] = r_luz[9];
        b[10] = id;
    }
    luz::poner_lateral(&mut b, luz::lateral(r_luz));
    luz::poner_dormir(&mut b, luz::dormir(r_luz));
    cambio |= guardar(t, &actual, &b, Some((&tabla, r_tabla)))?;
    Ok(cambio)
}

fn restore(t: &Teclado) -> Result<(), String> {
    listo(restaurar(t)?, &l("como estaba antes de chispa", "back to how it was before chispa"));
    a_mano();
    Ok(())
}

fn correr(args: &[String]) -> Result<(), String> {
    let cmd = args.first().map_or("", String::as_str);
    let resto = args.get(1..).unwrap_or(&[]);
    match cmd {
        "help" | "--help" | "-h" => {
            println!("{}", if es() { AYUDA_ES } else { AYUDA_EN });
            return Ok(());
        }
        "modes" => {
            for e in luz::EFECTOS {
                let mut extra = Vec::new();
                if e.velocidad { extra.push("speed"); }
                if e.color { extra.push("color"); }
                println!("{:<15}{}", e.nombre, extra.join(" "));
            }
            return Ok(());
        }
        "tui" => return tui::correr(),
        "keys" => {
            println!("{}", luz::TECLAS.iter().map(|k| k.0).collect::<Vec<_>>().join(" "));
            return Ok(());
        }
        // la última leída, al instante y sin hablarle al teclado (para menús)
        "battery" if resto.first().is_some_and(|a| a == "--cached") => {
            let (b, hace) = bateria_de_memoria().ok_or_else(|| l("todavía no se ha leído", "not read yet"))?;
            println!("{b} {}", if hace < 120 { l("ahorita", "just now") } else { format!("{} min", hace / 60) });
            return Ok(());
        }
        "dump" | "" | "status" | "mode" | "brightness" | "speed" | "color" | "key" | "look" | "off" | "on" | "side" | "restore" | "sleep" | "battery" => {}
        "wallpaper" => return wallpaper(resto),
        "music" => {
            sostener::terminar(true);
            return musica::correr(resto);
        }
        "hold" => return sostener::correr(),
        "video" => return video::correr(resto),
        "save" => {
            sostener::terminar(true);
            return Ok(());
        }
        _ => return Err(format!("{}: {cmd} (chispa help)", l("no conozco ese comando", "unknown command"))),
    }
    let uno = || resto.first().map(String::as_str).ok_or_else(|| l("falta el valor (chispa help)", "missing value (chispa help)"));
    // lo que está mal escrito se avisa sin tocar el teclado
    let previo = match cmd {
        "brightness" => Some(numero(uno()?, 9, "brightness")?),
        "speed" => Some(numero(uno()?, 4, "speed")?),
        _ => None,
    };
    let dormir = match (cmd, resto.first()) {
        ("sleep", Some(x)) => Some(medios_minutos(x)?),
        _ => None,
    };
    let lado = if cmd == "side" { Some(modo_lateral(uno()?)?) } else { None };
    let col = if cmd == "color" { Some(color_o_error(uno()?)?) } else { None };
    // lo que guarda otra cosa: primero que el de fondo guarde lo sostenido (él
    // necesita el candado, así que antes de abrir). Apagar se lo lleva consigo.
    let sostenido = match cmd {
        "off" => sostener::terminar(false),
        "mode" | "brightness" | "speed" | "key" | "on" | "side" | "sleep" | "restore" => {
            sostener::terminar(true);
            None
        }
        _ => None,
    };
    let t = Teclado::abrir()?;
    match cmd {
        "" | "status" => estado(&t),
        "dump" => dump(&t),
        "mode" => mode(&t, resto),
        "brightness" => ajustar(&t, previo, None),
        "speed" => ajustar(&t, None, previo),
        "color" => un_color(&t, col.unwrap_or_default(), uno()?),
        "key" => key(&t, resto),
        "look" => look(&t),
        "off" => off(&t, sostenido),
        "on" => on(&t),
        "side" => side(&t, lado.unwrap_or_default()),
        "sleep" => sleep_(&t, dormir),
        "battery" => battery(&t),
        _ => restore(&t),
    }
}

/// Lo que corre de fondo (sin terminal que lo enseñe).
fn de_fondo(args: &[String]) -> bool {
    matches!(args.first().map(String::as_str), Some("hold" | "video" | "music")) || args.iter().any(|a| a == "--auto")
}

/// Una línea en ~/.local/state/chispa/log, con la hora. Lo de fondo manda su
/// salida a /dev/null: sin esto, cuando algo falla nadie se entera (pasó con el
/// hold viejo que pintaba azul). Nunca lleva lo que tecleas, solo qué pasó. Al
/// pasar de 64 KB se queda con la mitad más nueva.
pub(crate) fn anotar(que: &str) {
    let f = estado_dir().join("log");
    let _ = std::fs::create_dir_all(estado_dir());
    if std::fs::metadata(&f).is_ok_and(|m| m.len() > 64 * 1024)
        && let Ok(t) = std::fs::read_to_string(&f)
    {
        let mitad = t.len() / 2;
        let corte = t[mitad..].find('\n').map_or(mitad, |i| mitad + i + 1);
        let _ = std::fs::write(&f, &t[corte..]);
    }
    let linea = format!("{} [{}] {que}\n", hora(), std::process::id());
    let _ = std::fs::OpenOptions::new().create(true).append(true).open(&f).and_then(|mut f| std::io::Write::write_all(&mut f, linea.as_bytes()));
}

/// "2026-11-07 13:05:09", hora local.
fn hora() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as libc::time_t;
    // SAFETY: localtime_r solo escribe en `tm`, que vive aquí.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return t.to_string();
    }
    format!("{}-{:02}-{:02} {:02}:{:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let fondo = de_fondo(&args);
    if fondo {
        anotar(&format!("empieza: chispa {}", args.join(" ")));
    }
    let r = correr(&args);
    if fondo {
        match &r {
            Ok(()) => anotar("termina"),
            Err(e) => anotar(&format!("error: {e}")),
        }
    }
    if let Err(e) = r {
        eprintln!("chispa: {e}");
        std::process::exit(1);
    }
}

/// Los dos bloques tal cual, en hexadecimal. Solo lee.
fn dump(t: &Teclado) -> Result<(), String> {
    for (nombre, b) in [("luz", leer_luz(t)?), ("tabla", leer_tabla(t)?)] {
        println!("{nombre}");
        for (i, fila) in b.chunks(16).enumerate() {
            let hex: Vec<String> = fila.iter().map(|x| format!("{x:02x}")).collect();
            println!("{:4}: {}", i * 16, hex.join(" "));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_fundido_que_toca() {
        let (r, b) = (luz::mapa_uniforme((255, 0, 0)), luz::mapa_uniforme((0, 0, 255)));
        let negro = luz::mapa_uniforme((0, 0, 0));
        // apagar: a negro
        assert_eq!(fundido(21, 0, Some(r.clone()), None), Some((r.clone(), negro.clone())));
        // prender: desde negro
        assert_eq!(fundido(0, 21, None, Some(b.clone())), Some((negro, b.clone())));
        // otro color
        assert_eq!(fundido(21, 21, Some(r.clone()), Some(b.clone())), Some((r.clone(), b)));
        // lo mismo (solo cambió la barrita): nada
        assert_eq!(fundido(21, 21, Some(r.clone()), Some(r.clone())), None);
        // un efecto que se mueve solo: no se sabe qué se ve, nada
        assert_eq!(fundido(3, 21, None, Some(r)), None);
    }
}
