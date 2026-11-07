//! Los bloques de la luz, sin tocar el teclado: el de ajustes (128 bytes) y la
//! tabla por tecla (384). Siempre se edita el bloque que regresó el teclado,
//! nunca se arma uno nuevo: ahí mismo viven el modo Mac, el polling y el sleep.

pub const LARGO_LUZ: usize = 128;
pub const LARGO_TABLA: usize = 384;
pub const LEDS: usize = 126;
pub type Rgb = (u8, u8, u8);
pub const PERSONAL: u8 = 21;

pub struct Efecto {
    pub id: u8,
    pub nombre: &'static str,
    pub velocidad: bool,
    pub color: bool,
}

const fn e(id: u8, nombre: &'static str, velocidad: bool, color: bool) -> Efecto {
    Efecto { id, nombre, velocidad, color }
}

/// Los números del firmware, como se ven en un F75 de verdad: los nombres de
/// la app oficial no les atinan.
pub const EFECTOS: &[Efecto] = &[
    e(1, "static", false, true),
    e(2, "breathing", true, true),
    e(3, "spectrum", true, false),
    e(4, "ripple-row", true, true),
    e(5, "raindrops", true, true),
    e(6, "wave", true, true),
    e(7, "ripple", true, true),
    e(8, "twinkle", true, true),
    e(10, "snake", true, true),
    e(11, "stream", true, true),
    e(12, "reactive", true, true),
    e(13, "center-wave", true, true),
    e(15, "rainbow-wheel", true, false),
    e(16, "waterfall", true, false),
    e(17, "center-flash", true, true),
    e(PERSONAL, "custom", false, false),
];

pub const COLORES: [(&str, (u8, u8, u8)); 7] = [
    ("red", (255, 0, 0)),
    ("green", (0, 255, 0)),
    ("blue", (0, 0, 255)),
    ("yellow", (255, 255, 0)),
    ("purple", (255, 0, 255)),
    ("cyan", (0, 255, 255)),
    ("white", (255, 255, 255)),
];
/// color 7 del efecto: el firmware usa todos los colores
pub const MULTI: u8 = 7;

/// Los colores con nombre que no son del firmware: sirven para la tabla, no
/// para los efectos (cuyo color es un índice a COLORES). El naranja lleva poco
/// verde porque en leds el verde pesa más que en pantalla.
pub const EXTRA: [(&str, (u8, u8, u8)); 2] = [("orange", (255, 80, 0)), ("pink", (255, 40, 120))];

/// Las muestras del selector, en orden de arcoíris.
pub const MUESTRAS: [(&str, (u8, u8, u8)); 9] = [
    ("red", (255, 0, 0)),
    ("orange", (255, 80, 0)),
    ("yellow", (255, 255, 0)),
    ("green", (0, 255, 0)),
    ("cyan", (0, 255, 255)),
    ("blue", (0, 0, 255)),
    ("purple", (255, 0, 255)),
    ("pink", (255, 40, 120)),
    ("white", (255, 255, 255)),
];

pub fn efecto(nombre: &str) -> Option<&'static Efecto> {
    EFECTOS.iter().find(|e| e.nombre == nombre)
}

pub fn efecto_id(id: u8) -> Option<&'static Efecto> {
    EFECTOS.iter().find(|e| e.id == id)
}

pub fn valido(b: &[u8]) -> bool {
    b.len() == LARGO_LUZ && b[126] == 0x5a && b[127] == 0xa5
}

fn lugar(id: u8) -> usize {
    56 + 2 * id as usize
}

pub fn activo(b: &[u8]) -> u8 {
    b[10]
}

/// (brillo 0-9, velocidad 0-4, color 0-7) del efecto
pub fn ajustes(b: &[u8], id: u8) -> (u8, u8, u8) {
    let o = lugar(id);
    (b[o], b[o + 1] >> 4, b[o + 1] & 0x0f)
}

/// Prende un efecto y cambia solo lo que se pidió de él.
pub fn poner(b: &mut [u8], id: u8, brillo: Option<u8>, velocidad: Option<u8>, color: Option<u8>) {
    b[9] = u8::from(id == PERSONAL);
    b[10] = id;
    let o = lugar(id);
    if let Some(x) = brillo {
        b[o] = x;
    }
    if let Some(x) = velocidad {
        b[o + 1] = (x << 4) | (b[o + 1] & 0x0f);
    }
    if let Some(x) = color {
        b[o + 1] = (b[o + 1] & 0xf0) | x;
    }
}

/// La barrita blanca junto al Esc (Fn+Shift izquierdo). Sus LEDs solo son
/// blancos: el color, brillo y velocidad que trae la app no le hacen nada.
pub const LATERAL: usize = 18;
pub const MODOS_LATERAL: [&str; 4] = ["off", "fill", "breathing", "static"];

pub fn lateral(b: &[u8]) -> u8 {
    b[LATERAL]
}

pub fn poner_lateral(b: &mut [u8], modo: u8) {
    if (modo as usize) < MODOS_LATERAL.len() {
        b[LATERAL] = modo;
    }
}

/// Cuánto espera el teclado sin teclear para apagar las luces (por 2.4 GHz o
/// Bluetooth), en medios minutos. 0 = nunca. De fábrica: 2 (1 minuto).
pub const DORMIR: usize = 24;
pub const MAX_DORMIR: u8 = 40;
/// Las que ofrece la app oficial, en medios minutos.
pub const OPCIONES_DORMIR: [u8; 10] = [0, 2, 3, 4, 6, 8, 10, 20, 30, 40];

pub fn dormir(b: &[u8]) -> u8 {
    b[DORMIR]
}

pub fn poner_dormir(b: &mut [u8], medios: u8) {
    if medios <= MAX_DORMIR {
        b[DORMIR] = medios;
    }
}

/// "1 min", "1.5 min"; None es nunca
pub fn minutos(medios: u8) -> Option<String> {
    (medios != 0).then(|| if medios.is_multiple_of(2) { format!("{} min", medios / 2) } else { format!("{}.5 min", medios / 2) })
}

pub fn apagar(b: &mut [u8]) {
    b[9] = 0;
    b[10] = 0;
}

/// La última revisión antes de escribir: solo cambió lo de la luz.
pub fn solo_luz(antes: &[u8], ahora: &[u8]) -> bool {
    if antes.len() != LARGO_LUZ || ahora.len() != LARGO_LUZ {
        return false;
    }
    let id = ahora[10];
    let o = lugar(id.max(1));
    let lateral_ok = (ahora[LATERAL] as usize) < MODOS_LATERAL.len();
    let dormir_ok = ahora[DORMIR] <= MAX_DORMIR;
    (0..LARGO_LUZ).all(|i| {
        antes[i] == ahora[i]
            || i == 9
            || i == 10
            || (i == LATERAL && lateral_ok)
            || (i == DORMIR && dormir_ok)
            || (id >= 1 && (i == o || i == o + 1))
    })
}

/// Los colores de dos tablas: lo que sigue después (6 bytes) solo existe por
/// cable, y por radio no se puede ni leer ni escribir.
/// De `a` a `b`; `x` 0..1. Por canal al cuadrado, que en LEDs se ve parejo.
pub fn mezclar(a: Rgb, b: Rgb, x: f32) -> Rgb {
    let m = |p: u8, q: u8| {
        let (p, q) = ((p as f32 / 255.0).powi(2), (q as f32 / 255.0).powi(2));
        ((p + (q - p) * x).sqrt() * 255.0).round() as u8
    };
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// Un color por tecla: (led, color), en el orden de TECLAS.
pub type Mapa = Vec<(u8, Rgb)>;

pub fn mapa_uniforme(c: Rgb) -> Mapa {
    TECLAS.iter().map(|k| (k.1 as u8, c)).collect()
}

pub fn mapa_de_tabla(tabla: &[u8]) -> Mapa {
    TECLAS
        .iter()
        .map(|k| (k.1 as u8, tabla.get(k.1).zip(tabla.get(LEDS + k.1)).zip(tabla.get(2 * LEDS + k.1)).map_or((0, 0, 0), |((r, g), b)| (*r, *g, *b))))
        .collect()
}

pub fn es_negro(m: &Mapa) -> bool {
    m.iter().all(|(_, c)| *c == (0, 0, 0))
}

/// Lo que se ve tecla por tecla: todo negro apagado, la tabla con el efecto de
/// la tabla. Los demás efectos se mueven solos: None.
pub fn visible_mapa(luz: &[u8], tabla: &[u8]) -> Option<Mapa> {
    match activo(luz) {
        0 => Some(mapa_uniforme((0, 0, 0))),
        PERSONAL if tabla.len() >= 3 * LEDS => Some(mapa_de_tabla(tabla)),
        _ => None,
    }
}

/// Tecla por tecla de un mapa a otro; `x` 0..1.
pub fn mezclar_mapa(de: &Mapa, a: &Mapa, x: f32) -> Mapa {
    a.iter()
        .map(|(led, c)| {
            let d = de.iter().find(|(l, _)| l == led).map_or(*c, |t| t.1);
            (*led, mezclar(d, *c, x))
        })
        .collect()
}

/// Grupos de teclas del mismo color para un cuadro en vivo. Por radio un
/// cuadro aguanta 126 bytes: se juntan en 10 colores (10 × 4 + 80 = 120).
pub fn grupos(m: &Mapa, radio: bool) -> Vec<(Rgb, Vec<u8>)> {
    if radio { juntar(m, 10) } else { agrupar(m) }
}

fn agrupar(m: &Mapa) -> Vec<(Rgb, Vec<u8>)> {
    let mut g: Vec<(Rgb, Vec<u8>)> = Vec::new();
    for (led, c) in m {
        match g.iter_mut().find(|x| x.0 == *c) {
            Some(x) => x.1.push(*led),
            None => g.push((*c, vec![*led])),
        }
    }
    g
}

/// Junta los colores en `k` grupos (corte por la mediana).
pub fn juntar(teclas: &Mapa, k: usize) -> Vec<(Rgb, Vec<u8>)> {
    let canal = |c: Rgb, ch: usize| [c.0, c.1, c.2][ch];
    let mut cajas: Vec<Mapa> = vec![teclas.clone()];
    while cajas.len() < k {
        // la caja con más rango en algún canal
        let rango = |c: &Mapa| -> (u8, usize) {
            (0..3)
                .map(|ch| {
                    let v = c.iter().map(|t| canal(t.1, ch));
                    (v.clone().max().unwrap_or(0) - v.min().unwrap_or(0), ch)
                })
                .max()
                .unwrap_or((0, 0))
        };
        let Some((i, (r, ch))) = cajas.iter().enumerate().filter(|(_, c)| c.len() > 1).map(|(i, c)| (i, rango(c))).max_by_key(|(_, r)| r.0) else { break };
        if r == 0 {
            break;
        }
        let mut c = cajas.swap_remove(i);
        c.sort_by_key(|t| canal(t.1, ch));
        let otra = c.split_off(c.len() / 2);
        cajas.push(c);
        cajas.push(otra);
    }
    cajas
        .into_iter()
        .filter(|c| !c.is_empty())
        .map(|c| {
            let n = c.len() as u32;
            let s = c.iter().fold([0u32; 3], |s, t| [s[0] + t.1.0 as u32, s[1] + t.1.1 as u32, s[2] + t.1.2 as u32]);
            (((s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8), c.iter().map(|t| t.0).collect())
        })
        .collect()
}



pub fn misma_tabla(a: &[u8], b: &[u8]) -> bool {
    a.len() >= 3 * LEDS && b.len() >= 3 * LEDS && a[..3 * LEDS] == b[..3 * LEDS]
}

pub fn pintar(tabla: &mut [u8], led: usize, (r, g, b): (u8, u8, u8)) {
    tabla[led] = r;
    tabla[LEDS + led] = g;
    tabla[2 * LEDS + led] = b;
}

/// Nombre de la tecla → su led.
pub const TECLAS: &[(&str, usize)] = &[
    ("esc", 0), ("f1", 12), ("f2", 18), ("f3", 24), ("f4", 30), ("f5", 36), ("f6", 42), ("f7", 48),
    ("f8", 54), ("f9", 60), ("f10", 66), ("f11", 72), ("f12", 78), ("del", 85), ("pgup", 86), ("pgdn", 87),
    ("backtick", 1), ("1", 7), ("2", 13), ("3", 19), ("4", 25), ("5", 31), ("6", 37), ("7", 43), ("8", 49),
    ("9", 55), ("0", 61), ("minus", 67), ("equal", 73), ("backspace", 79),
    ("tab", 2), ("q", 8), ("w", 14), ("e", 20), ("r", 26), ("t", 32), ("y", 38), ("u", 44), ("i", 50),
    ("o", 56), ("p", 62), ("lbracket", 68), ("rbracket", 74), ("backslash", 80),
    ("capslock", 3), ("a", 9), ("s", 15), ("d", 21), ("f", 27), ("g", 33), ("h", 39), ("j", 45), ("k", 51),
    ("l", 57), ("semicolon", 63), ("quote", 69), ("enter", 81),
    ("lshift", 4), ("z", 10), ("x", 16), ("c", 22), ("v", 28), ("b", 34), ("n", 40), ("m", 46),
    ("comma", 52), ("dot", 58), ("slash", 64), ("rshift", 70), ("up", 82), ("end", 88),
    ("lctrl", 5), ("lwin", 11), ("lalt", 17), ("space", 35), ("fn", 53), ("rctrl", 59),
    ("left", 77), ("down", 83), ("right", 89),
];

/// Para el look: el color oscurecido, para que se sigan viendo las luces
/// blancas que prende el firmware al apretar Fn.
pub fn oscurecer((r, g, b): (u8, u8, u8)) -> (u8, u8, u8) {
    let f = |x: u8| (x as u16 * 35 / 100) as u8;
    (f(r), f(g), f(b))
}

/// h 0-360, s y v 0-1
pub fn hsv_a_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let f = |x: f32| ((x + m) * 255.0).round() as u8;
    (f(r), f(g), f(b))
}

pub fn rgb_a_hsv((r, g, b): (u8, u8, u8)) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, if max == 0.0 { 0.0 } else { d / max }, max)
}

/// Contraste WCAG entre dos colores: de 1 (iguales) a 21 (negro y blanco).
pub fn contraste(a: (u8, u8, u8), b: (u8, u8, u8)) -> f32 {
    let lum = |(r, g, b): (u8, u8, u8)| {
        let f = |x: u8| {
            let x = x as f32 / 255.0;
            if x <= 0.03928 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// Debajo de esto es un gris: subirlo inventaría un color que el wallpaper no tiene.
const SATURACION_COLOR: f32 = 0.15;

/// De la paleta de un wallpaper, el color para los leds: el de menos contraste con
/// la base del wallpaper (color0), ya subido como se va a ver para que no salga
/// deslavado. Los grises no compiten: subirlos inventaría un color. Si son puros grises, el más vivo; y si de veras no hay
/// color, un blanco a media luz.
pub fn color_de_paleta(paleta: &[(u8, u8, u8)]) -> Option<(u8, u8, u8)> {
    let base = *paleta.first()?;
    // Custom (dany): saturación completa. Con 0.75 los otros dos canales quedan a
    // un cuarto y en los LEDs eso se ve como blanco revuelto: deslavado
    let subir = |(h, _, _): (f32, f32, f32)| hsv_a_rgb(h, 1.0, 1.0);
    let hsv = || paleta.iter().map(|c| rgb_a_hsv(*c));
    let con_color = hsv().filter(|c| c.1 >= SATURACION_COLOR).map(subir).min_by(|a, b| contraste(*a, base).total_cmp(&contraste(*b, base)));
    if con_color.is_some() {
        return con_color;
    }
    let (h, s, v) = hsv().max_by(|a, b| (a.1 * a.2).total_cmp(&(b.1 * b.2)))?;
    Some(if s < 0.08 { hsv_a_rgb(h, s, 0.6) } else { subir((h, s, v)) })
}

/// El texto de discada.conf con KEYBOARD_LIGHT cambiado, respetando lo demás.
pub fn con_modo(texto: &str, modo: &str) -> String {
    let mut hallado = false;
    let mut filas: Vec<String> = texto
        .lines()
        .map(|f| match f.strip_prefix("KEYBOARD_LIGHT=") {
            Some(resto) => {
                hallado = true;
                let comentario = resto.find('#').map_or("", |i| &resto[i..]);
                if comentario.is_empty() {
                    format!("KEYBOARD_LIGHT={modo}")
                } else {
                    format!("KEYBOARD_LIGHT={modo}  {comentario}")
                }
            }
            None => f.to_string(),
        })
        .collect();
    if !hallado {
        filas.push(format!("KEYBOARD_LIGHT={modo}  # wallpaper | custom: la luz del AULA F75 (chispa)"));
    }
    filas.join("\n") + "\n"
}

pub fn modo_de(texto: &str) -> Option<String> {
    texto.lines().find_map(|f| {
        let v = f.strip_prefix("KEYBOARD_LIGHT=")?.split('#').next()?.trim();
        (!v.is_empty()).then(|| v.to_string())
    })
}

pub fn color(s: &str) -> Option<(u8, u8, u8)> {
    if let Some((_, c)) = COLORES.iter().chain(EXTRA.iter()).find(|(n, _)| *n == s) {
        return Some(*c);
    }
    if s == "black" || s == "off" {
        return Some((0, 0, 0));
    }
    let h = s.strip_prefix('#').unwrap_or(s);
    if !h.is_ascii() || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let n = |t: &str| u8::from_str_radix(t, 16).ok();
    match h.len() {
        3 => {
            let d = |i: usize| n(&h[i..i + 1]).map(|x| x * 17);
            Some((d(0)?, d(1)?, d(2)?))
        }
        6 => Some((n(&h[0..2])?, n(&h[2..4])?, n(&h[4..6])?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// el bloque de un F75 sano, leído del teclado
    const SANO: [u8; 128] = [
        0x00, 0x03, 0x03, 0x02, 0x00, 0x00, 0x04, 0x04, 0x07, 0x00, 0x03, 0x20, 0x01, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x01, 0x00, 0x02, 0x01, 0x00, 0xff, 0x02, 0x00, 0x01, 0x00, 0x01, 0x00, 0x03, 0x01,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x81, 0xff, 0x09, 0x37, 0x09, 0x37, 0x09, 0x37,
        0x09, 0x37, 0x09, 0x37, 0x08, 0x37, 0x09, 0x37, 0x09, 0x37, 0x09, 0x37, 0x09, 0x37, 0x09, 0x37,
        0x09, 0x37, 0x09, 0x37, 0x09, 0x37, 0x09, 0x37, 0x09, 0x37, 0x09, 0x37, 0x09, 0x37, 0x09, 0x37,
        0x09, 0x37, 0x07, 0x47, 0x07, 0x47, 0x07, 0x44, 0x07, 0x44, 0x07, 0x44, 0x07, 0x44, 0x07, 0x44,
        0x07, 0x44, 0x07, 0x44, 0x04, 0x09, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x5a, 0xa5,
    ];

    #[test]
    fn lee_el_bloque_de_verdad() {
        assert!(valido(&SANO));
        assert_eq!(activo(&SANO), 3);
        assert_eq!(ajustes(&SANO, 3), (9, 3, 7));
        assert_eq!(ajustes(&SANO, 6), (8, 3, 7));
    }

    #[test]
    fn poner_cambia_solo_lo_pedido() {
        let mut b = SANO;
        poner(&mut b, 6, Some(5), Some(1), Some(2));
        assert_eq!((b[9], b[10], b[68], b[69]), (0, 6, 5, 0x12));
        let cambios: Vec<usize> = (0..128).filter(|&i| b[i] != SANO[i]).collect();
        assert_eq!(cambios, [10, 68, 69]);
        assert!(solo_luz(&SANO, &b));
    }

    #[test]
    fn el_personal_prende_su_bandera() {
        let mut b = SANO;
        poner(&mut b, PERSONAL, None, None, None);
        assert_eq!((b[9], b[10]), (1, 21));
        assert!(solo_luz(&SANO, &b));
    }

    #[test]
    fn solo_luz_no_deja_pasar_lo_demas() {
        for i in [0usize, 1, 8, 24, 56, 126, 127] {
            let mut b = SANO;
            b[i] ^= 0xff;
            assert!(!solo_luz(&SANO, &b), "byte {i}");
        }
        // otro efecto que no es el activo
        let mut b = SANO;
        b[58] = 1;
        assert!(!solo_luz(&SANO, &b));
    }

    #[test]
    fn la_barrita_solo_acepta_sus_cuatro_modos() {
        let mut b = SANO;
        poner_lateral(&mut b, 0);
        assert_eq!(lateral(&b), 0);
        assert!(solo_luz(&SANO, &b));
        let cambios: Vec<usize> = (0..128).filter(|&i| b[i] != SANO[i]).collect();
        assert_eq!(cambios, [LATERAL]);
        poner_lateral(&mut b, 9);
        assert_eq!(lateral(&b), 0, "un modo que no existe no se pone");
        b[LATERAL] = 9;
        assert!(!solo_luz(&SANO, &b));
    }

    #[test]
    fn el_sueno_va_de_nunca_a_20_minutos() {
        let mut b = SANO;
        poner_dormir(&mut b, 0);
        assert!(solo_luz(&SANO, &b));
        poner_dormir(&mut b, 41);
        assert_eq!(dormir(&b), 0, "más de 20 minutos no se pone");
        b[DORMIR] = 41;
        assert!(!solo_luz(&SANO, &b));
        assert_eq!(minutos(3).as_deref(), Some("1.5 min"));
        assert_eq!(minutos(0), None);
    }

    #[test]
    fn mezclar_llega_a_los_dos_extremos() {
        assert_eq!(mezclar((255, 0, 0), (0, 0, 255), 0.0), (255, 0, 0));
        assert_eq!(mezclar((255, 0, 0), (0, 0, 255), 1.0), (0, 0, 255));
        let (r, _, b) = mezclar((255, 0, 0), (0, 0, 255), 0.5);
        assert!(r > 127 && b > 127, "a la mitad no se oscurece");
    }

    #[test]
    fn juntar_no_pasa_del_tope_ni_pierde_teclas() {
        let teclas: Mapa = (0..80u8).map(|i| (i, (i * 3, 255 - i * 2, i))).collect();
        let g = grupos(&teclas, true);
        assert!(g.len() <= 10);
        let mut leds: Vec<u8> = g.iter().flat_map(|x| x.1.clone()).collect();
        leds.sort();
        assert_eq!(leds, (0..80u8).collect::<Vec<_>>());
        let bytes: usize = g.iter().map(|x| 4 + x.1.len()).sum();
        assert!(bytes <= 126, "{bytes}");
        // por cable, cada color tal cual
        assert_eq!(grupos(&teclas, false).len(), 80);
    }

    #[test]
    fn un_solo_color_es_un_solo_grupo() {
        let m = mapa_uniforme((10, 20, 30));
        assert_eq!(grupos(&m, true).len(), 1);
        assert_eq!(grupos(&m, false).len(), 1);
    }

    #[test]
    fn el_mapa_de_la_tabla_va_y_viene() {
        let mut t = vec![0u8; LARGO_TABLA];
        for (led, c) in mapa_uniforme((1, 2, 3)) {
            pintar(&mut t, led as usize, c);
        }
        assert_eq!(mapa_de_tabla(&t), mapa_uniforme((1, 2, 3)));
    }

    #[test]
    fn apagar_no_toca_ajustes() {
        let mut b = SANO;
        apagar(&mut b);
        assert_eq!((b[9], b[10]), (0, 0));
        assert!(solo_luz(&SANO, &b));
        assert_eq!(b[58..126], SANO[58..126]);
    }

    #[test]
    fn oscurecer_deja_el_tono() {
        assert_eq!(oscurecer((0xb9, 0xb6, 0xd8)), (64, 63, 75));
        assert_eq!(oscurecer((0, 0, 0)), (0, 0, 0));
        assert_eq!(oscurecer((255, 255, 255)), (89, 89, 89));
    }

    #[test]
    fn el_naranja_no_cambia_los_colores_del_firmware() {
        assert_eq!(color("orange"), Some((255, 80, 0)));
        assert_eq!(COLORES.len(), 7);
        assert!(COLORES.iter().all(|c| c.0 != "orange"));
    }

    #[test]
    fn hsv_de_ida_y_vuelta() {
        assert_eq!(hsv_a_rgb(0.0, 1.0, 1.0), (255, 0, 0));
        assert_eq!(hsv_a_rgb(120.0, 1.0, 1.0), (0, 255, 0));
        assert_eq!(hsv_a_rgb(240.0, 1.0, 1.0), (0, 0, 255));
        assert_eq!(hsv_a_rgb(360.0, 1.0, 1.0), (255, 0, 0));
        assert_eq!(hsv_a_rgb(10.0, 0.0, 1.0), (255, 255, 255));
        assert_eq!(hsv_a_rgb(10.0, 1.0, 0.0), (0, 0, 0));
        for c in [(0x2a, 0x27, 0x40), (255, 80, 0), (1, 2, 3), (200, 200, 200), (0, 0, 0), (12, 250, 99)] {
            let (h, s, v) = rgb_a_hsv(c);
            assert_eq!(hsv_a_rgb(h, s, v), c);
        }
    }

    #[test]
    fn de_la_paleta_sale_el_de_menos_contraste_y_subido() {
        // la del wallpaper de dany: puros azules grisáceos
        let p = [(0x48, 0x46, 0x44), (0x6c, 0x87, 0x92), (0xe0, 0xdd, 0xd4), (0x9d, 0x9a, 0x94)];
        let (r, g, b) = color_de_paleta(&p).unwrap();
        let (h, s, v) = rgb_a_hsv((r, g, b));
        assert!((190.0..205.0).contains(&h), "tono {h}");
        assert!(s >= 0.74 && v == 1.0, "{s} {v}");
        // sobre base oscura, ya subidos, el rojo contrasta menos que el cian
        let p = [(0x20, 0x20, 0x20), (0xd5, 0x58, 0x50), (0x62, 0x96, 0x96)];
        let (r, g, b) = color_de_paleta(&p).unwrap();
        assert!(r == 255 && g < 100, "{r} {g} {b}");
        // sobre base clara gana el más claro de los subidos
        let p = [(0xf0, 0xf0, 0xf0), (0xd5, 0x58, 0x50), (0x62, 0x96, 0x96)];
        let (r, g, b) = color_de_paleta(&p).unwrap();
        assert!(r < 100 && g > 200 && b > 200, "{r} {g} {b}");
        // un casi blanco es gris: no compite
        let p = [(0x10, 0x10, 0x10), (0xf0, 0xf0, 0xee), (0xb0, 0x30, 0x30)];
        let (r, g, b) = color_de_paleta(&p).unwrap();
        assert!(r == 255 && g < 100 && b < 100, "{r} {g} {b}");
        assert_eq!(color_de_paleta(&[]), None);
    }

    #[test]
    fn una_paleta_gris_no_inventa_color() {
        let (r, g, b) = color_de_paleta(&[(30, 30, 30), (200, 200, 200)]).unwrap();
        assert_eq!((r, g, b), (153, 153, 153));
    }

    #[test]
    fn el_modo_se_cambia_sin_tocar_lo_demas() {
        let base = "# ajustes\nLANGUAGE=en        # es | en\nFONT_SIZE=-2\n";
        let a = con_modo(base, "wallpaper");
        assert!(a.starts_with(base));
        assert_eq!(modo_de(&a).as_deref(), Some("wallpaper"));
        let b = con_modo(&a, "custom");
        assert_eq!(modo_de(&b).as_deref(), Some("custom"));
        assert_eq!(b.matches("KEYBOARD_LIGHT=").count(), 1);
        assert!(b.contains("# wallpaper | custom"));
        assert!(b.contains("LANGUAGE=en        # es | en"));
        assert_eq!(modo_de(base), None);
    }

    #[test]
    fn colores() {
        assert_eq!(color("#b9b6d8"), Some((0xb9, 0xb6, 0xd8)));
        assert_eq!(color("fff"), Some((255, 255, 255)));
        assert_eq!(color("red"), Some((255, 0, 0)));
        assert_eq!(color("ñññ"), None);
        assert_eq!(color("#12"), None);
    }

    #[test]
    fn teclas_sin_repetir() {
        let mut leds: Vec<usize> = TECLAS.iter().map(|t| t.1).collect();
        leds.sort();
        leds.dedup();
        assert_eq!(leds.len(), TECLAS.len());
        assert!(leds.iter().all(|&l| l < LEDS));
    }

    #[test]
    fn pintar_va_en_planos() {
        let mut t = vec![0u8; LARGO_TABLA];
        pintar(&mut t, 9, (1, 2, 3));
        assert_eq!((t[9], t[126 + 9], t[252 + 9]), (1, 2, 3));
    }
}
