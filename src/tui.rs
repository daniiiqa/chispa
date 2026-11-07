//! `chispa tui`: la luz en una ventana, con el teclado dibujado. Nunca le
//! escribe al teclado por cada tecla apretada: los cambios se juntan y salen
//! un rato después de soltar, y no se le pregunta nada mientras tanto.

use crate::hid::Teclado;
use crate::luz::{self, EFECTOS, LARGO_TABLA, LEDS, MULTI, PERSONAL};
use crate::{anotar_ultimo, color_del_look, cambiar_mapa, guardar, hay_programa, l, texto_bateria, leer_luz, leer_tabla, poner_modo, respaldo, restaurar, sigue_wallpaper, ultimo};
use disco::look::Look;
use ratatui::Frame;
use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::crossterm::execute;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use std::cell::RefCell;
use std::time::{Duration, Instant};

/// cuánto esperar sin cambios antes de escribir
const ESPERA: Duration = Duration::from_millis(800);

type Rgb = (u8, u8, u8);

/// (lo que se ve, nombre en luz::TECLAS, ancho)
const FILAS: [&[(&str, &str, u16)]; 6] = [
    &[("esc", "esc", 4), ("f1", "f1", 4), ("f2", "f2", 4), ("f3", "f3", 4), ("f4", "f4", 4), ("f5", "f5", 4), ("f6", "f6", 4), ("f7", "f7", 4), ("f8", "f8", 4), ("f9", "f9", 4), ("f10", "f10", 4), ("f11", "f11", 4), ("f12", "f12", 4)],
    &[("`", "backtick", 4), ("1", "1", 4), ("2", "2", 4), ("3", "3", 4), ("4", "4", 4), ("5", "5", 4), ("6", "6", 4), ("7", "7", 4), ("8", "8", 4), ("9", "9", 4), ("0", "0", 4), ("-", "minus", 4), ("=", "equal", 4), ("bksp", "backspace", 8), ("del", "del", 4)],
    &[("tab", "tab", 6), ("q", "q", 4), ("w", "w", 4), ("e", "e", 4), ("r", "r", 4), ("t", "t", 4), ("y", "y", 4), ("u", "u", 4), ("i", "i", 4), ("o", "o", 4), ("p", "p", 4), ("[", "lbracket", 4), ("]", "rbracket", 4), ("\\", "backslash", 6), ("pgup", "pgup", 4)],
    &[("caps", "capslock", 7), ("a", "a", 4), ("s", "s", 4), ("d", "d", 4), ("f", "f", 4), ("g", "g", 4), ("h", "h", 4), ("j", "j", 4), ("k", "k", 4), ("l", "l", 4), (";", "semicolon", 4), ("'", "quote", 4), ("enter", "enter", 10), ("pgdn", "pgdn", 4)],
    &[("shift", "lshift", 9), ("z", "z", 4), ("x", "x", 4), ("c", "c", 4), ("v", "v", 4), ("b", "b", 4), ("n", "n", 4), ("m", "m", 4), (",", "comma", 4), (".", "dot", 4), ("/", "slash", 4), ("shift", "rshift", 8), ("up", "up", 4), ("end", "end", 4)],
    &[("ctrl", "lctrl", 5), ("win", "lwin", 5), ("alt", "lalt", 5), ("space", "space", 34), ("fn", "fn", 4), ("ctrl", "rctrl", 5), ("lf", "left", 4), ("dn", "down", 4), ("rt", "right", 4)],
];

fn led(nombre: &str) -> usize {
    luz::TECLAS.iter().find(|k| k.0 == nombre).map_or(0, |k| k.1)
}

/// Cada tecla en el teclado de verdad: (led, x, y), las dos de 0 a 1, del
/// centro de la tecla. Para pintar el teclado como una pantallita.
pub(crate) fn posiciones() -> Vec<(u8, f32, f32)> {
    let ancho = (0..FILAS.len()).map(|f| inicio(f, FILAS[f].len())).max().unwrap_or(1) as f32;
    let mut v = Vec::new();
    for (f, fila) in FILAS.iter().enumerate() {
        for (c, k) in fila.iter().enumerate() {
            let x = (inicio(f, c) as f32 + k.2 as f32 / 2.0) / ancho;
            let y = (f as f32 + 0.5) / FILAS.len() as f32;
            v.push((led(k.1) as u8, x.min(1.0), y));
        }
    }
    v
}

/// dónde empieza la tecla en su fila, en celdas
fn inicio(fila: usize, col: usize) -> u16 {
    FILAS[fila][..col].iter().map(|k| k.2 + 1).sum()
}

/// Los renglones de ajustes
const BRILLO: usize = 0;
const VELOCIDAD: usize = 1;
const COLOR: usize = 2;
const LATERAL: usize = 3;
const SUENO: usize = 4;

/// Lo que hay debajo del ratón. Se anota al dibujar, así lo que se ve y lo que
/// se puede picar nunca se desacomodan.
#[derive(Clone, Copy, PartialEq)]
enum Clic {
    Panel(Panel),
    Efecto(usize),
    /// el renglón de un ajuste (la rueda lo mueve)
    Fila(usize),
    /// un valor exacto de un ajuste
    Ajuste(usize, u8),
    Tecla(usize, usize),
    Brocha,
    /// como si se apretara esa tecla
    Boton(KeyCode),
    Selector,
}

fn dentro_de(r: Rect, x: u16, y: u16) -> bool {
    x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
}

#[derive(Clone, Copy, PartialEq)]
enum Panel {
    Efectos,
    Ajustes,
    Teclas,
}

/// El selector de color: muestras, y tono, saturación y brillo.
#[derive(Clone, PartialEq)]
struct Selector {
    /// 0 muestras, 1 tono, 2 saturación, 3 brillo
    fila: usize,
    muestra: usize,
    h: f32,
    s: f32,
    v: f32,
}

impl Selector {
    fn new(c: Rgb) -> Selector {
        let (h, s, v) = luz::rgb_a_hsv(c);
        let muestra = luz::MUESTRAS.iter().position(|m| m.1 == c).unwrap_or(0);
        Selector { fila: 0, muestra, h, s, v }
    }

    fn rgb(&self) -> Rgb {
        luz::hsv_a_rgb(self.h, self.s, self.v)
    }

    fn escoger(&mut self, i: usize) {
        self.muestra = i.min(luz::MUESTRAS.len() - 1);
        (self.h, self.s, self.v) = luz::rgb_a_hsv(luz::MUESTRAS[self.muestra].1);
    }

    fn mover(&mut self, d: i32) {
        match self.fila {
            0 => self.escoger((self.muestra as i32 + d).clamp(0, luz::MUESTRAS.len() as i32 - 1) as usize),
            1 => self.h = (self.h + 6.0 * d as f32).clamp(0.0, 359.0),
            2 => self.s = (self.s + 0.04 * d as f32).clamp(0.0, 1.0),
            _ => self.v = (self.v + 0.04 * d as f32).clamp(0.0, 1.0),
        }
    }

    /// `f` 0..1 a lo largo de la barra de la fila
    fn poner(&mut self, fila: usize, f: f32) {
        self.fila = fila;
        match fila {
            1 => self.h = f * 359.0,
            2 => self.s = f,
            _ => self.v = f,
        }
    }
}

const ANCHO_SELECTOR: u16 = 52;
const ALTO_SELECTOR: u16 = 11;
const ROTULO: u16 = 12;

fn caja_selector(a: Rect) -> Rect {
    let w = ANCHO_SELECTOR.min(a.width);
    let h = ALTO_SELECTOR.min(a.height);
    Rect::new(a.x + (a.width - w) / 2, a.y + (a.height - h) / 2, w, h)
}

#[derive(PartialEq)]
enum Modo {
    Normal,
    /// escogiendo el color de la brocha
    Selector(Selector),
    /// escribiendo el color en hexadecimal
    Color(String),
    /// esperando la confirmación de restore
    Restaurar,
}

pub struct App {
    /// lo que tiene el teclado, y lo que se quiere que tenga
    luz: Vec<u8>,
    tabla: Vec<u8>,
    b_luz: Vec<u8>,
    b_tabla: Vec<u8>,
    panel: Panel,
    efecto: usize,
    ajuste: usize,
    fila: usize,
    col: usize,
    brocha: Rgb,
    modo: Modo,
    /// cuándo se tocó lo último que falta escribir
    pendiente: Option<Instant>,
    /// lo que hay que decirle a dany
    aviso: String,
    /// el teclado dejó de contestar: nada más hasta que `u` lo vuelva a encontrar
    mudo: bool,
    /// por dónde está conectado, para la cabecera
    enlace: String,
    /// el teclado sigue al wallpaper
    sigue: bool,
    /// la batería al abrir o releer (solo por radio)
    bateria: Option<String>,
    /// lo pendiente se escogió a mano: deja de seguir al wallpaper al guardarse
    manual: bool,
    /// lo que se puede ofrecer: lo que no, ni se enseña
    hay_wallust: bool,
    hay_look: bool,
    hay_cava: bool,
    /// lo que se puede picar, del último dibujo
    zonas: RefCell<Vec<(Rect, Clic)>>,
    salir: bool,
}

impl App {
    fn new(luz_: Vec<u8>, tabla: Vec<u8>) -> App {
        let activo = luz::activo(&luz_);
        App {
            b_luz: luz_.clone(),
            b_tabla: tabla.clone(),
            luz: luz_,
            tabla,
            panel: Panel::Efectos,
            efecto: EFECTOS.iter().position(|e| e.id == activo).unwrap_or(0),
            ajuste: 0,
            fila: 3,
            col: 1,
            brocha: (0x2a, 0x27, 0x40),
            modo: Modo::Normal,
            pendiente: None,
            aviso: String::new(),
            mudo: false,
            enlace: String::new(),
            sigue: false,
            bateria: None,
            manual: false,
            hay_wallust: false,
            hay_look: false,
            hay_cava: false,
            zonas: RefCell::new(Vec::new()),
            salir: false,
        }
    }

    fn zona(&self, r: Rect, c: Clic) {
        if r.width > 0 && r.height > 0 {
            self.zonas.borrow_mut().push((r, c));
        }
    }

    /// Los ajustes que tiene el efecto de ahora; la barrita siempre.
    fn visibles(&self) -> Vec<usize> {
        let id = self.activo();
        let e = luz::efecto_id(id);
        let mut v = Vec::new();
        if id != 0 {
            v.push(BRILLO);
        }
        if e.is_some_and(|e| e.velocidad) {
            v.push(VELOCIDAD);
        }
        if e.is_some_and(|e| e.color) {
            v.push(COLOR);
        }
        v.push(LATERAL);
        v.push(SUENO);
        v
    }

    /// el ajuste elegido, si es que se ve; si no, el primero que se vea
    fn ajuste_visible(&self) -> usize {
        let v = self.visibles();
        if v.contains(&self.ajuste) { self.ajuste } else { v[0] }
    }

    fn activo(&self) -> u8 {
        luz::activo(&self.b_luz)
    }

    fn tecla_actual(&self) -> &'static (&'static str, &'static str, u16) {
        &FILAS[self.fila][self.col]
    }

    fn color_tecla(&self, n: usize) -> Rgb {
        (self.b_tabla[n], self.b_tabla[LEDS + n], self.b_tabla[2 * LEDS + n])
    }

    fn tocar(&mut self, ahora: Instant) {
        self.pendiente = Some(ahora);
    }

    /// ya pasó el rato sin cambios
    fn vencio(&self, ahora: Instant) -> bool {
        self.pendiente.is_some_and(|t| ahora.duration_since(t) >= ESPERA)
    }

    /// ±1 al ajuste elegido, en borrador
    fn mover_ajuste(&mut self, d: i8, ahora: Instant) {
        let paso = move |v: u8, max: u8| (v as i16 + d as i16).clamp(0, max as i16) as u8;
        self.ajustar(self.ajuste_visible(), paso, ahora);
    }

    /// Cambia un ajuste en borrador: `nuevo(valor, tope)`.
    fn ajustar(&mut self, cual: usize, nuevo: impl Fn(u8, u8) -> u8, ahora: Instant) {
        let antes = self.b_luz.clone();
        if cual == SUENO {
            // se camina por las opciones de la app, no de medio en medio minuto
            let op = luz::OPCIONES_DORMIR;
            let ahora_ = luz::dormir(&self.b_luz);
            let i = op.iter().position(|m| *m >= ahora_).unwrap_or(op.len() - 1);
            let j = nuevo(i as u8, op.len() as u8 - 1) as usize;
            luz::poner_dormir(&mut self.b_luz, op[j.min(op.len() - 1)]);
        } else if cual == LATERAL {
            let v = nuevo(luz::lateral(&self.b_luz), luz::MODOS_LATERAL.len() as u8 - 1);
            luz::poner_lateral(&mut self.b_luz, v);
        } else if self.visibles().contains(&cual) {
            let id = self.activo();
            let (br, sp, co) = luz::ajustes(&self.b_luz, id);
            match cual {
                BRILLO => luz::poner(&mut self.b_luz, id, Some(nuevo(br, 9)), None, None),
                VELOCIDAD => luz::poner(&mut self.b_luz, id, None, Some(nuevo(sp, 4)), None),
                _ => luz::poner(&mut self.b_luz, id, None, None, Some(nuevo(co, MULTI))),
            }
        }
        // dejar apretada la flecha en el tope también aguanta la escritura
        if self.b_luz != antes || self.pendiente.is_some() {
            self.tocar(ahora);
        }
    }

    fn pintar(&mut self, leds: &[usize], c: Rgb, ahora: Instant) {
        for n in leds {
            luz::pintar(&mut self.b_tabla, *n, c);
        }
        // la tabla solo se ve con su efecto
        luz::poner(&mut self.b_luz, PERSONAL, None, None, None);
        self.efecto = EFECTOS.iter().position(|e| e.id == PERSONAL).unwrap_or(0);
        self.manual = true;
        self.tocar(ahora);
    }

    fn mover_tecla(&mut self, dx: i16, dy: i16) {
        if dy != 0 {
            // a la tecla que quede más cerca arriba o abajo
            let x = inicio(self.fila, self.col) + self.tecla_actual().2 / 2;
            self.fila = (self.fila as i16 + dy).clamp(0, FILAS.len() as i16 - 1) as usize;
            self.col = (0..FILAS[self.fila].len())
                .min_by_key(|&c| (inicio(self.fila, c) + FILAS[self.fila][c].2 / 2).abs_diff(x))
                .unwrap_or(0);
        } else {
            self.col = (self.col as i16 + dx).clamp(0, FILAS[self.fila].len() as i16 - 1) as usize;
        }
    }

    /// Lo que no toca el teclado. Regresa qué falta que sí lo toque.
    fn tecla_normal(&mut self, k: KeyEvent, ahora: Instant) -> Option<Accion> {
        let ctrl_c = k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c');
        if ctrl_c || matches!(k.code, KeyCode::Char('q') | KeyCode::Esc) {
            self.salir = true;
            return None;
        }
        match k.code {
            KeyCode::Tab => {
                self.panel = match self.panel {
                    Panel::Efectos => Panel::Ajustes,
                    Panel::Ajustes => Panel::Teclas,
                    Panel::Teclas => Panel::Efectos,
                };
            }
            KeyCode::BackTab => {
                self.panel = match self.panel {
                    Panel::Efectos => Panel::Teclas,
                    Panel::Ajustes => Panel::Efectos,
                    Panel::Teclas => Panel::Ajustes,
                };
            }
            KeyCode::Char('c') => self.modo = Modo::Selector(Selector::new(self.brocha)),
            KeyCode::Char('a') => {
                let todas: Vec<usize> = luz::TECLAS.iter().map(|t| t.1).collect();
                self.pintar(&todas, self.brocha, ahora);
            }
            KeyCode::Char('l') => match color_del_look() {
                Ok(c) => {
                    self.brocha = c;
                    let todas: Vec<usize> = luz::TECLAS.iter().map(|t| t.1).collect();
                    self.pintar(&todas, c, ahora);
                }
                Err(e) => self.aviso = e,
            },
            KeyCode::Char('o') => return Some(Accion::Prender),
            KeyCode::Char('w') => return Some(Accion::Wallpaper),
            KeyCode::Char('m') if self.hay_cava => return Some(Accion::Musica),
            KeyCode::Char('s') if crate::sostener::sostenido().is_some() => return Some(Accion::Guardar),
            KeyCode::Char('r') => self.modo = Modo::Restaurar,
            KeyCode::Char('u') => return Some(Accion::Releer),
            _ => match self.panel {
                Panel::Efectos => match k.code {
                    KeyCode::Up | KeyCode::Char('k') => self.efecto = self.efecto.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => self.efecto = (self.efecto + 1).min(EFECTOS.len() - 1),
                    KeyCode::Enter => return Some(Accion::Efecto(EFECTOS[self.efecto].id)),
                    _ => {}
                },
                Panel::Ajustes => match k.code {
                    KeyCode::Up | KeyCode::Char('k') | KeyCode::Down | KeyCode::Char('j') => {
                        let v = self.visibles();
                        let i = v.iter().position(|x| *x == self.ajuste_visible()).unwrap_or(0);
                        let abajo = matches!(k.code, KeyCode::Down | KeyCode::Char('j'));
                        self.ajuste = v[if abajo { (i + 1).min(v.len() - 1) } else { i.saturating_sub(1) }];
                    }
                    KeyCode::Left => self.mover_ajuste(-1, ahora),
                    KeyCode::Right => self.mover_ajuste(1, ahora),
                    _ => {}
                },
                Panel::Teclas => match k.code {
                    KeyCode::Up => self.mover_tecla(0, -1),
                    KeyCode::Down => self.mover_tecla(0, 1),
                    KeyCode::Left => self.mover_tecla(-1, 0),
                    KeyCode::Right => self.mover_tecla(1, 0),
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        let n = led(self.tecla_actual().1);
                        self.pintar(&[n], self.brocha, ahora);
                    }
                    _ => {}
                },
            },
        }
        None
    }

    fn tecla_selector(&mut self, k: KeyEvent, ahora: Instant) {
        let Modo::Selector(sel) = &mut self.modo else { return };
        match k.code {
            KeyCode::Esc => self.modo = Modo::Normal,
            KeyCode::Up => sel.fila = sel.fila.saturating_sub(1),
            KeyCode::Down => sel.fila = (sel.fila + 1).min(3),
            KeyCode::Left => sel.mover(-1),
            KeyCode::Right => sel.mover(1),
            KeyCode::Char('#') => self.modo = Modo::Color(String::from("#")),
            // desde el teclado se escoge la brocha para pintar tecla por tecla;
            // desde lo demás, escoger un color es querer verlo en el teclado
            KeyCode::Enter | KeyCode::Char('a') => {
                self.brocha = sel.rgb();
                self.modo = Modo::Normal;
                if k.code == KeyCode::Enter && self.panel == Panel::Teclas {
                    self.aviso = l("brocha lista: enter pinta la tecla", "brush ready: enter paints the key");
                } else {
                    let todas: Vec<usize> = luz::TECLAS.iter().map(|t| t.1).collect();
                    self.pintar(&todas, self.brocha, ahora);
                }
            }
            _ => {}
        }
    }

    /// Una tecla, según en qué se anda.
    fn tecla(&mut self, k: KeyEvent, ahora: Instant) -> Option<Accion> {
        match self.modo {
            Modo::Selector(_) => self.tecla_selector(k, ahora),
            Modo::Color(_) => self.tecla_color(k),
            Modo::Restaurar => {
                self.modo = Modo::Normal;
                if k.code == KeyCode::Char('y') {
                    return Some(Accion::Restaurar);
                }
            }
            Modo::Normal => return self.tecla_normal(k, ahora),
        }
        None
    }

    /// El ratón: clic elige o pinta, arrastrar pinta o mueve barras, la rueda
    /// sube y baja, clic derecho en una tecla toma su color para la brocha.
    fn raton(&mut self, m: MouseEvent, pantalla: Rect, ahora: Instant) -> Option<Accion> {
        let hay = self.zonas.borrow().iter().rev().find(|(r, _)| dentro_de(*r, m.column, m.row)).map(|z| z.1);
        let clic = matches!(m.kind, MouseEventKind::Down(MouseButton::Left));
        let arrastre = matches!(m.kind, MouseEventKind::Drag(MouseButton::Left));
        let boton = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
        match (&self.modo, hay) {
            (_, Some(Clic::Boton(c))) if clic => return self.tecla(boton(c), ahora),
            (Modo::Selector(_), Some(Clic::Selector)) => self.raton_selector(m, pantalla),
            // arrastrando una barra se puede salir de la caja sin cerrarla
            (Modo::Selector(_), _) if arrastre => self.raton_selector(m, pantalla),
            (Modo::Selector(_), _) if clic => self.modo = Modo::Normal,
            (Modo::Normal, Some(z)) => return self.raton_normal(m, z, ahora),
            _ => {}
        }
        None
    }

    fn raton_normal(&mut self, m: MouseEvent, z: Clic, ahora: Instant) -> Option<Accion> {
        let clic = matches!(m.kind, MouseEventKind::Down(MouseButton::Left));
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left) => match z {
                Clic::Panel(p) if clic => self.panel = p,
                Clic::Efecto(i) if clic => {
                    self.panel = Panel::Efectos;
                    self.efecto = i;
                    return Some(Accion::Efecto(EFECTOS[i].id));
                }
                Clic::Fila(c) if clic => {
                    self.panel = Panel::Ajustes;
                    self.ajuste = c;
                }
                Clic::Ajuste(c, v) => {
                    self.panel = Panel::Ajustes;
                    self.ajuste = c;
                    self.ajustar(c, |_, _| v, ahora);
                }
                Clic::Tecla(f, c) => {
                    self.panel = Panel::Teclas;
                    (self.fila, self.col) = (f, c);
                    let n = led(self.tecla_actual().1);
                    if self.color_tecla(n) != self.brocha || self.activo() != PERSONAL {
                        self.pintar(&[n], self.brocha, ahora);
                    }
                }
                Clic::Brocha if clic => self.modo = Modo::Selector(Selector::new(self.brocha)),
                _ => {}
            },
            MouseEventKind::Down(MouseButton::Right) => {
                if let Clic::Tecla(f, c) = z {
                    (self.fila, self.col) = (f, c);
                    self.brocha = self.color_tecla(led(self.tecla_actual().1));
                    let (r, g, b) = self.brocha;
                    self.aviso = format!("{}: #{r:02x}{g:02x}{b:02x}", l("brocha", "brush"));
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let d: i8 = if m.kind == MouseEventKind::ScrollUp { 1 } else { -1 };
                match z {
                    Clic::Efecto(_) | Clic::Panel(Panel::Efectos) => {
                        self.panel = Panel::Efectos;
                        self.efecto = (self.efecto as i16 - d as i16).clamp(0, EFECTOS.len() as i16 - 1) as usize;
                    }
                    Clic::Fila(c) | Clic::Ajuste(c, _) => {
                        self.panel = Panel::Ajustes;
                        self.ajuste = c;
                        self.mover_ajuste(d, ahora);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        None
    }

    fn raton_selector(&mut self, m: MouseEvent, pantalla: Rect) {
        if !matches!(m.kind, MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left)) {
            return;
        }
        let Modo::Selector(sel) = &mut self.modo else { return };
        let c = caja_selector(pantalla);
        let (x, y) = (c.x + 1, c.y + 1);
        let barra = c.width.saturating_sub(2 + ROTULO + 1).max(1);
        match m.row.checked_sub(y) {
            Some(0 | 1) if m.column > x => sel.escoger(((m.column - x - 1) / 5) as usize),
            Some(f @ 3..=5) if m.column >= x + ROTULO => {
                let n = (m.column - x - ROTULO).min(barra - 1);
                sel.poner(f as usize - 2, n as f32 / (barra - 1).max(1) as f32);
            }
            _ => {}
        }
    }

    fn tecla_color(&mut self, k: KeyEvent) {
        let Modo::Color(s) = &mut self.modo else { return };
        match k.code {
            KeyCode::Esc => self.modo = Modo::Normal,
            KeyCode::Backspace => {
                s.pop();
            }
            KeyCode::Enter => match luz::color(s.trim()) {
                Some(c) => {
                    self.brocha = c;
                    self.modo = Modo::Normal;
                }
                None => self.aviso = l("no entiendo ese color", "unknown color"),
            },
            KeyCode::Char(c) if s.len() < 12 => s.push(c),
            _ => {}
        }
    }

    /// Manda el borrador al teclado.
    fn escribir(&mut self) {
        self.pendiente = None;
        if self.mudo {
            self.b_luz = self.luz.clone();
            self.b_tabla = self.tabla.clone();
            return;
        }
        // solo cambiaron colores de teclas (una o todas): sin parpadeo, en vivo
        let solo_efecto = (0..self.luz.len()).all(|i| self.luz[i] == self.b_luz[i] || i == 9 || i == 10);
        if solo_efecto && luz::activo(&self.b_luz) == PERSONAL && self.b_tabla != self.tabla {
            let m = luz::mapa_de_tabla(&self.b_tabla);
            match Teclado::abrir().and_then(|t| cambiar_mapa(&t, &m).inspect_err(|_| self.mudo = t.colgado())) {
                Ok(_) => {
                    self.luz = self.b_luz.clone();
                    self.tabla = self.b_tabla.clone();
                    self.aviso = l("listo (se guarda al bloquear la pantalla)", "done (saved when the screen locks)");
                    self.dejar_de_seguir();
                }
                Err(e) => {
                    self.b_luz = self.luz.clone();
                    self.b_tabla = self.tabla.clone();
                    self.aviso = e;
                }
            }
            return;
        }
        // lo demás se guarda: primero que el de fondo guarde lo que sostiene
        crate::sostener::terminar(true);
        let r = Teclado::abrir().and_then(|t| {
            // la tabla va siempre, aunque no cambie: con ella se funde al prender y apagar
            let tabla = Some((self.tabla.as_slice(), self.b_tabla.as_slice()));
            guardar(&t, &self.luz, &self.b_luz, tabla).inspect_err(|_| self.mudo = t.colgado())
        });
        match r {
            Ok(_) => {
                self.luz = self.b_luz.clone();
                self.tabla = self.b_tabla.clone();
                self.aviso = l("guardado", "saved");
                self.dejar_de_seguir();
            }
            Err(e) => {
                self.b_luz = self.luz.clone();
                self.b_tabla = self.tabla.clone();
                self.aviso = e;
            }
        }
    }

    fn dejar_de_seguir(&mut self) {
        if std::mem::take(&mut self.manual) && (self.sigue || crate::en_musica()) {
            match poner_modo("custom") {
                Ok(()) => self.sigue = false,
                Err(e) => self.aviso = e,
            }
        }
    }

    fn accion(&mut self, a: Accion) {
        // primero lo que falte: cada escritura cambia un solo efecto
        if self.pendiente.is_some() && !matches!(a, Accion::Releer) {
            self.escribir();
        }
        if self.mudo && !matches!(a, Accion::Releer) {
            return;
        }
        match a {
            Accion::Efecto(id) => {
                luz::poner(&mut self.b_luz, id, None, None, None);
                self.manual = true;
                self.escribir();
            }
            Accion::Wallpaper => {
                let m = match crate::mapa_del_wallpaper(None) {
                    Ok(m) => m,
                    Err(e) => {
                        self.aviso = e;
                        return;
                    }
                };
                self.releer_con(|t| cambiar_mapa(t, &m).map(|_| ()));
                if !self.mudo && self.aviso == l("listo", "done") {
                    match poner_modo("wallpaper") {
                        Ok(()) => self.sigue = true,
                        Err(e) => self.aviso = e,
                    }
                }
            }
            Accion::Musica => {
                if crate::en_musica() {
                    // el visualizador se va solo al ver otro modo
                    match poner_modo("custom") {
                        Ok(()) => self.aviso = l("visualizador apagado", "visualizer off"),
                        Err(e) => self.aviso = e,
                    }
                } else {
                    let yo = std::env::current_exe().map_err(|e| e.to_string());
                    let r = yo.and_then(|yo| {
                        use std::os::unix::process::CommandExt;
                        std::process::Command::new(yo)
                            .arg("music")
                            .stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .process_group(0)
                            .spawn()
                            .map_err(|e| e.to_string())
                    });
                    self.aviso = match r {
                        Ok(_) => l("visualizador prendido: pon música", "visualizer on: play some music"),
                        Err(e) => e,
                    };
                }
                self.sigue = sigue_wallpaper();
            }
            Accion::Guardar => {
                crate::sostener::terminar(true);
                self.releer_con(|_| Ok(()));
                if !self.mudo {
                    self.aviso = l("guardado en el teclado", "saved on the keyboard");
                }
            }
            Accion::Prender => {
                let id = self.activo();
                if id != 0 {
                    if let Err(e) = anotar_ultimo(id) {
                        self.aviso = e;
                        return;
                    }
                    luz::apagar(&mut self.b_luz);
                } else if let Some(u) = ultimo() {
                    luz::poner(&mut self.b_luz, u, None, None, None);
                } else {
                    self.aviso = l("no sé cuál estaba: escoge un efecto", "don't know which was on: pick an effect");
                    return;
                }
                self.escribir();
            }
            Accion::Restaurar => {
                crate::sostener::terminar(true);
                self.manual = true;
                self.releer_con(|t| restaurar(t).map(|_| ()));
                self.dejar_de_seguir();
            }
            Accion::Releer => self.releer_con(|_| Ok(())),
        }
    }

    /// Hace algo con el teclado y vuelve a leerlo todo.
    fn releer_con(&mut self, f: impl FnOnce(&Teclado) -> Result<(), String>) {
        self.pendiente = None;
        let r = Teclado::abrir().and_then(|t| {
            let leido = f(&t).and_then(|()| Ok((leer_luz(&t)?, leer_tabla(&t)?)));
            if leido.is_ok() {
                self.bateria = texto_bateria(&t).or(self.bateria.take());
            }
            self.mudo = t.colgado();
            leido
        });
        match r.map(|(a, b)| con_sostenido(a, b)) {
            Ok((a, b)) => {
                self.efecto = EFECTOS.iter().position(|e| e.id == luz::activo(&a)).unwrap_or(self.efecto);
                self.b_luz = a.clone();
                self.b_tabla = b.clone();
                self.luz = a;
                self.tabla = b;
                self.aviso = l("listo", "done");
            }
            Err(e) => {
                self.b_luz = self.luz.clone();
                self.b_tabla = self.tabla.clone();
                self.aviso = e;
            }
        }
    }
}

enum Accion {
    Efecto(u8),
    Prender,
    Wallpaper,
    Restaurar,
    Releer,
    /// prender o apagar el visualizador
    Musica,
    /// guardar ya lo que está en vivo (parpadea)
    Guardar,
}

/// Un renglón hecho de pedazos, y cada pedazo con lo que pasa si se le pica.
/// Las zonas se recortan a `area` (el renglón dentro de su caja).
fn renglon(app: &App, area: Rect, pedazos: Vec<(Span<'static>, Option<Clic>)>) -> Line<'static> {
    let mut x = area.x;
    let mut spans = Vec::with_capacity(pedazos.len());
    for (sp, clic) in pedazos {
        let w = sp.width() as u16;
        if let Some(c) = clic {
            app.zona(Rect::new(x, area.y, w, 1).intersection(area), c);
        }
        x = x.saturating_add(w);
        spans.push(sp);
    }
    Line::from(spans)
}

/// Un botón: la tecla resaltada y lo que hace, los dos se pueden picar.
fn boton(look: &Look, tecla: &str, que: &str, c: KeyCode) -> Vec<(Span<'static>, Option<Clic>)> {
    let t = look.theme;
    vec![
        (Span::styled(format!(" {tecla} "), Style::new().fg(t.base).bg(t.primary).add_modifier(Modifier::BOLD)), Some(Clic::Boton(c))),
        (Span::styled(format!(" {} ", look.tx(que)), Style::new().fg(t.fg)), Some(Clic::Boton(c))),
        (Span::raw(" "), None),
    ]
}

/// Los botones de abajo. Lo que no se puede hacer ahorita no sale.
fn botones(app: &App, look: &Look) -> Vec<(Span<'static>, Option<Clic>)> {
    let mut v = Vec::new();
    let mut b = |tecla: &str, que: String, c: KeyCode| v.extend(boton(look, tecla, &que, c));
    match &app.modo {
        Modo::Selector(_) => {
            b("enter", l("aplicar", "apply"), KeyCode::Enter);
            b("#", l("escribir", "type hex"), KeyCode::Char('#'));
            b("esc", l("cancelar", "cancel"), KeyCode::Esc);
        }
        Modo::Color(_) => {
            b("enter", l("listo", "ok"), KeyCode::Enter);
            b("esc", l("cancelar", "cancel"), KeyCode::Esc);
        }
        Modo::Restaurar => {
            b("y", l("sí", "yes"), KeyCode::Char('y'));
            b("n", "no".into(), KeyCode::Char('n'));
        }
        Modo::Normal => {
            b("c", "color".into(), KeyCode::Char('c'));
            b("a", l("todas", "all"), KeyCode::Char('a'));
            if app.hay_wallust {
                b("w", "wallpaper".into(), KeyCode::Char('w'));
            }
            if app.hay_cava {
                let que = if crate::en_musica() { l("sin música", "music off") } else { l("música", "music") };
                b("m", que, KeyCode::Char('m'));
            }
            if crate::sostener::sostenido().is_some() {
                b("s", l("guardar", "save"), KeyCode::Char('s'));
            }
            if app.hay_look {
                b("l", "look".into(), KeyCode::Char('l'));
            }
            if app.activo() != 0 {
                b("o", l("apagar", "off"), KeyCode::Char('o'));
            } else if ultimo().is_some() {
                b("o", l("prender", "on"), KeyCode::Char('o'));
            }
            if respaldo().exists() {
                b("r", "restore".into(), KeyCode::Char('r'));
            }
            b("u", l("releer", "reread"), KeyCode::Char('u'));
            b("q", l("salir", "quit"), KeyCode::Char('q'));
        }
    }
    v
}

fn dibujar(f: &mut Frame, app: &App, look: &Look) {
    app.zonas.borrow_mut().clear();
    let t = look.theme;
    if t.bg != Color::Reset {
        f.render_widget(ratatui::widgets::Block::new().style(Style::new().bg(t.bg)), f.area());
    }
    let [cab, medio, teclado, ayuda] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(8),
        Constraint::Length(8),
        Constraint::Length(2),
    ])
    .areas(f.area());
    let [ef, aj] = Layout::horizontal([Constraint::Length(26), Constraint::Min(30)]).areas(medio);

    let id = app.activo();
    let actual = if id == 0 {
        l("apagado", "off")
    } else {
        luz::efecto_id(id).map_or_else(|| id.to_string(), |e| e.nombre.to_string())
    };
    let mut cabeza = vec![
        Span::styled(format!(" {} ", look.tx("chispa")), Style::new().fg(t.base).bg(t.primary).add_modifier(Modifier::BOLD)),
        Span::styled(format!("  {actual}"), Style::new().fg(t.fg)),
    ];
    if !app.enlace.is_empty() {
        cabeza.push(Span::styled(format!("  · {}", app.enlace), Style::new().fg(t.muted)));
    }
    if let Some(b) = &app.bateria {
        cabeza.push(Span::styled(format!("  · {} {b}", l("batería", "battery")), Style::new().fg(t.muted)));
    }
    let modo = match crate::modo_actual().as_deref() {
        Some("wallpaper") => Some(l("sigue al wallpaper", "follows wallpaper")),
        Some("music") => Some(l("visualizador", "visualizer")),
        _ => None,
    };
    if let Some(m) = modo {
        // por cable no hay en vivo: el visualizador y el video se pausan
        let pausa = if crate::hid::hay_cable() && (crate::en_musica() || crate::video::hay_mpvpaper()) {
            format!(" ({})", l("en pausa por cable", "paused on cable"))
        } else {
            String::new()
        };
        cabeza.push(Span::styled(format!("  · {m}{pausa}"), Style::new().fg(t.muted)));
    }
    if let Some((_, fps, ms)) = crate::video::medida() {
        cabeza.push(Span::styled(format!("  · video {fps:.0} fps ({ms:.0} ms)"), Style::new().fg(t.muted)));
    } else if crate::sostener::sostenido().is_some() {
        cabeza.push(Span::styled(format!("  · {}", l("en vivo, sin guardar", "live, not saved yet")), Style::new().fg(t.muted)));
    }
    if app.pendiente.is_some() {
        cabeza.push(Span::styled(format!("  {}", l("sin guardar…", "unsaved…")), Style::new().fg(t.muted)));
    }
    f.render_widget(Paragraph::new(Line::from(cabeza)), cab);

    efectos(f, ef, app, look);
    ajustes(f, aj, app, look);
    dibujar_teclado(f, teclado, app, look);
    if let Modo::Selector(sel) = &app.modo {
        selector(f, app, sel, look);
    }

    let (txt, c) = match &app.modo {
        Modo::Selector(_) => (l("↑↓ fila · ←→ o ratón cambiar", "↑↓ row · ←→ or mouse change"), t.muted),
        Modo::Color(s) => (format!("{}: {s}▏", l("color (#rrggbb o nombre)", "color (#rrggbb or name)")), t.primary),
        Modo::Restaurar => (l("¿regresar a como estaba antes de chispa?", "go back to how it was before chispa?"), t.primary),
        Modo::Normal if !app.aviso.is_empty() => (app.aviso.clone(), if app.mudo { t.red } else { t.fg }),
        Modo::Normal => (
            l("tab panel · enter elegir/pintar · clic derecho en una tecla: tomar su color", "tab panel · enter pick/paint · right-click a key: take its color"),
            t.muted,
        ),
    };
    let [msg, bot] = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(ayuda);
    f.render_widget(Paragraph::new(Line::styled(look.tx(&txt), Style::new().fg(c))), msg);
    let b = renglon(app, bot, botones(app, look));
    f.render_widget(Paragraph::new(b), bot);
}

fn selector(f: &mut Frame, app: &App, sel: &Selector, look: &Look) {
    let t = look.theme;
    let caja = caja_selector(f.area());
    app.zona(caja, Clic::Selector);
    f.render_widget(Clear, caja);
    let bloque = look.block(true).title(look.tx(&l(" color ", " color "))).style(Style::new().bg(t.base));
    let dentro = bloque.inner(caja);
    f.render_widget(bloque, caja);
    let fondo = Style::new().fg(t.fg).bg(t.base);
    let rgb = |c: Rgb| Color::Rgb(c.0, c.1, c.2);

    let mut muestras = vec![Span::styled(" ", fondo)];
    for (i, (_, c)) in luz::MUESTRAS.iter().enumerate() {
        let elegida = sel.muestra == i && *c == sel.rgb();
        let marca = if elegida { "▾▾▾▾" } else { "    " };
        let claro = 299 * c.0 as u32 + 587 * c.1 as u32 + 114 * c.2 as u32 > 128_000;
        muestras.push(Span::styled(marca, Style::new().bg(rgb(*c)).fg(if claro { Color::Black } else { Color::White })));
        muestras.push(Span::styled(" ", fondo));
    }
    let nombre = luz::MUESTRAS.iter().find(|m| m.1 == sel.rgb()).map_or("—", |m| m.0);
    let foco0 = if sel.fila == 0 { look.sel(true).0 } else { fondo };

    let ancho = dentro.width.saturating_sub(ROTULO + 1).max(1) as usize;
    let barra = |fila: usize, rotulo: String, valor: f32, color_en: &dyn Fn(f32) -> Rgb| {
        let estilo = if sel.fila == fila { look.sel(true).0 } else { fondo };
        let mut sp = vec![Span::styled(format!(" {:<w$}", look.tx(&rotulo), w = ROTULO as usize - 1), estilo)];
        let puesto = (valor * (ancho - 1) as f32).round() as usize;
        for i in 0..ancho {
            let c = color_en(i as f32 / (ancho - 1).max(1) as f32);
            if i == puesto {
                let claro = 299 * c.0 as u32 + 587 * c.1 as u32 + 114 * c.2 as u32 > 128_000;
                sp.push(Span::styled("┃", Style::new().bg(rgb(c)).fg(if claro { Color::Black } else { Color::White }).add_modifier(Modifier::BOLD)));
            } else {
                sp.push(Span::styled(" ", Style::new().bg(rgb(c))));
            }
        }
        Line::from(sp)
    };

    let (r, g, b) = sel.rgb();
    let filas = vec![
        Line::from(muestras),
        Line::from(vec![Span::styled(format!(" {:<w$}", look.tx(nombre), w = (dentro.width as usize).saturating_sub(1)), foco0)]),
        Line::styled("", fondo),
        barra(1, l("tono", "hue"), sel.h / 359.0, &|x| luz::hsv_a_rgb(x * 359.0, 1.0, 1.0)),
        barra(2, l("intensidad", "saturation"), sel.s, &|x| luz::hsv_a_rgb(sel.h, x, sel.v.max(0.3))),
        barra(3, l("brillo", "brightness"), sel.v, &|x| luz::hsv_a_rgb(sel.h, sel.s, x)),
        Line::styled("", fondo),
        renglon(
            app,
            Rect::new(dentro.x, dentro.y + 7, dentro.width, u16::from(dentro.height > 7)),
            vec![
                (Span::styled(" ", fondo), None),
                (Span::styled("        ", Style::new().bg(Color::Rgb(r, g, b))), Some(Clic::Boton(KeyCode::Enter))),
                (Span::styled(format!("  #{r:02x}{g:02x}{b:02x}   "), fondo), None),
                (
                    Span::styled(format!(" {} ", look.tx(&l("aplicar", "apply"))), Style::new().fg(t.base).bg(t.primary).add_modifier(Modifier::BOLD)),
                    Some(Clic::Boton(KeyCode::Enter)),
                ),
            ],
        ),
    ];
    f.render_widget(Paragraph::new(filas).style(fondo), dentro);
}

fn efectos(f: &mut Frame, a: Rect, app: &App, look: &Look) {
    let foco = app.panel == Panel::Efectos;
    let bloque = look.block(foco).title(look.tx(&l(" efecto ", " effect ")));
    let dentro = bloque.inner(a);
    f.render_widget(bloque, a);
    app.zona(a, Clic::Panel(Panel::Efectos));
    let alto = dentro.height as usize;
    let desde = app.efecto.saturating_sub(alto.saturating_sub(1));
    let (sel, marca) = look.sel(foco);
    let filas: Vec<Line> = EFECTOS
        .iter()
        .enumerate()
        .skip(desde)
        .take(alto)
        .enumerate()
        .map(|(n, (i, e))| {
            app.zona(Rect::new(dentro.x, dentro.y + n as u16, dentro.width, 1), Clic::Efecto(i));
            let puesto = if e.id == app.activo() { "●" } else { " " };
            let txt = format!("{puesto} {}", look.tx(e.nombre));
            if i == app.efecto {
                Line::styled(format!("{marca}{txt:<22}"), sel)
            } else {
                Line::styled(format!(" {txt}"), Style::new().fg(look.theme.fg))
            }
        })
        .collect();
    f.render_widget(Paragraph::new(filas), dentro);
}

fn ajustes(f: &mut Frame, a: Rect, app: &App, look: &Look) {
    let foco = app.panel == Panel::Ajustes;
    let bloque = look.block(foco).title(look.tx(&l(" ajustes ", " settings ")));
    let dentro = bloque.inner(a);
    f.render_widget(bloque, a);
    app.zona(a, Clic::Panel(Panel::Ajustes));
    let t = look.theme;
    let id = app.activo();
    let (br, sp, co) = if id == 0 { (0, 0, 0) } else { luz::ajustes(&app.b_luz, id) };
    let ancho = (dentro.width as usize).saturating_sub(24).clamp(6, 40);
    let rgb = |c: Rgb| Color::Rgb(c.0, c.1, c.2);
    // cada celda de la barra es el valor que queda si se le pica
    let barra = |cual: usize, v: u8, max: u8| {
        let (lleno, resto) = look.bar(v as f64 / max as f64, ancho);
        let mut p: Vec<(Span<'static>, Option<Clic>)> = lleno
            .chars()
            .map(|ch| (ch, t.primary))
            .chain(resto.chars().map(|ch| (ch, t.border)))
            .enumerate()
            .map(|(i, (ch, c))| {
                let valor = (((i + 1) * max as usize) as f32 / ancho as f32).round() as u8;
                (Span::styled(ch.to_string(), Style::new().fg(c)), Some(Clic::Ajuste(cual, valor)))
            })
            .collect();
        p.push((Span::styled(format!(" {v}/{max}"), Style::new().fg(t.fg)), Some(Clic::Fila(cual))));
        p
    };
    let nombres = [l("brillo", "brightness"), l("velocidad", "speed"), "color".to_string(), l("barrita", "side light"), l("dormir", "sleep")];
    let elegido = app.ajuste_visible();
    let mut filas = Vec::new();
    for (n, cual) in app.visibles().into_iter().enumerate() {
        let estilo = if foco && elegido == cual { look.sel(true).0 } else { Style::new().fg(t.fg) };
        let mut p = vec![(Span::styled(format!(" {:<12}", look.tx(&nombres[cual])), estilo), Some(Clic::Fila(cual)))];
        match cual {
            BRILLO => p.extend(barra(cual, br, 9)),
            VELOCIDAD => p.extend(barra(cual, sp, 4)),
            COLOR => {
                // los 7 colores del efecto y el multicolor, a picar
                for i in 0..=MULTI {
                    let marca = if co == i { "•" } else { " " };
                    if i == MULTI {
                        for (j, c) in [(255, 0, 0), (0, 255, 0), (0, 0, 255)].into_iter().enumerate() {
                            let ch = if j == 1 { marca } else { " " };
                            p.push((Span::styled(ch, Style::new().bg(rgb(c)).fg(Color::White)), Some(Clic::Ajuste(COLOR, i))));
                        }
                    } else {
                        let c = luz::COLORES[i as usize].1;
                        let claro = 299 * c.0 as u32 + 587 * c.1 as u32 + 114 * c.2 as u32 > 128_000;
                        let tinta = if claro { Color::Black } else { Color::White };
                        p.push((Span::styled(format!(" {marca} "), Style::new().bg(rgb(c)).fg(tinta)), Some(Clic::Ajuste(COLOR, i))));
                    }
                    p.push((Span::raw(" "), None));
                }
            }
            SUENO => {
                let actual = luz::dormir(&app.b_luz);
                for (i, m) in luz::OPCIONES_DORMIR.iter().enumerate() {
                    let nombre = luz::minutos(*m).map_or_else(|| l("nunca", "never"), |x| x.replace(" min", "m"));
                    let st = if actual == *m { Style::new().fg(t.base).bg(t.primary).add_modifier(Modifier::BOLD) } else { Style::new().fg(t.muted) };
                    p.push((Span::styled(format!(" {} ", look.tx(&nombre)), st), Some(Clic::Ajuste(SUENO, i as u8))));
                }
            }
            _ => {
                let actual = luz::lateral(&app.b_luz);
                for (i, m) in luz::MODOS_LATERAL.iter().enumerate() {
                    let nombre = match *m {
                        "off" => l("apagada", "off"),
                        "fill" => l("llenar", "fill"),
                        "breathing" => l("respirar", "breathing"),
                        _ => l("fija", "static"),
                    };
                    let st = if actual as usize == i { Style::new().fg(t.base).bg(t.primary).add_modifier(Modifier::BOLD) } else { Style::new().fg(t.muted) };
                    p.push((Span::styled(format!(" {} ", look.tx(&nombre)), st), Some(Clic::Ajuste(LATERAL, i as u8))));
                    p.push((Span::raw(" "), None));
                }
            }
        }
        let area = Rect::new(dentro.x, dentro.y + n as u16, dentro.width, u16::from((n as u16) < dentro.height));
        filas.push(renglon(app, area, p));
    }
    filas.push(Line::raw(""));
    let (r, g, b) = app.brocha;
    let y = filas.len() as u16;
    let area = Rect::new(dentro.x, dentro.y + y, dentro.width, u16::from(y < dentro.height));
    filas.push(renglon(
        app,
        area,
        vec![
            (Span::styled(format!(" {:<12}", look.tx(&l("brocha", "brush"))), Style::new().fg(t.fg)), Some(Clic::Brocha)),
            (Span::styled("    ", Style::new().bg(Color::Rgb(r, g, b))), Some(Clic::Brocha)),
            (Span::styled(format!(" #{r:02x}{g:02x}{b:02x}"), Style::new().fg(t.muted)), Some(Clic::Brocha)),
        ],
    ));
    f.render_widget(Paragraph::new(filas), dentro);
}

fn dibujar_teclado(f: &mut Frame, a: Rect, app: &App, look: &Look) {
    let foco = app.panel == Panel::Teclas;
    let titulo = if foco {
        format!(" {} ", app.tecla_actual().1)
    } else {
        look.tx(&l(" teclado ", " keyboard "))
    };
    let bloque = look.block(foco).title(titulo);
    let dentro = bloque.inner(a);
    f.render_widget(bloque, a);
    app.zona(a, Clic::Panel(Panel::Teclas));
    let mut filas = Vec::with_capacity(FILAS.len());
    for (nf, fila) in FILAS.iter().enumerate() {
        let mut sp = vec![(Span::raw(" "), None)];
        for (nc, (txt, nombre, ancho)) in fila.iter().enumerate() {
            let (r, g, b) = app.color_tecla(led(nombre));
            let claro = 299 * r as u32 + 587 * g as u32 + 114 * b as u32 > 128_000;
            let tinta = if claro { Color::Black } else { Color::White };
            let mut estilo = Style::new().bg(Color::Rgb(r, g, b)).fg(tinta);
            if foco && nf == app.fila && nc == app.col {
                estilo = estilo.add_modifier(Modifier::REVERSED | Modifier::BOLD);
            }
            sp.push((Span::styled(format!("{txt:^w$}", w = *ancho as usize), estilo), Some(Clic::Tecla(nf, nc))));
            sp.push((Span::raw(" "), None));
        }
        let area = Rect::new(dentro.x, dentro.y + nf as u16, dentro.width, u16::from((nf as u16) < dentro.height));
        filas.push(renglon(app, area, sp));
    }
    f.render_widget(Paragraph::new(filas), dentro);
}

/// Lo sostenido en vivo es lo que se ve: se enseña así, como si ya estuviera guardado.
fn con_sostenido(mut a: Vec<u8>, mut b: Vec<u8>) -> (Vec<u8>, Vec<u8>) {
    if let Some(m) = crate::sostener::sostenido() {
        for (led, c) in m {
            luz::pintar(&mut b, led as usize, c);
        }
        luz::poner(&mut a, PERSONAL, None, None, None);
    }
    (a, b)
}

pub fn correr() -> Result<(), String> {
    // lo primero, y sin abrir la pantalla: si no hay teclado se dice en la shell
    let (a, b, enlace) = {
        let t = Teclado::abrir()?;
        (leer_luz(&t)?, leer_tabla(&t)?, (t.enlace().nombre(), texto_bateria(&t)))
    };
    if b.len() != LARGO_TABLA {
        return Err(l("tabla de colores rara", "unexpected color table"));
    }
    let (a, b) = con_sostenido(a, b);
    let mut app = App::new(a, b);
    app.sigue = sigue_wallpaper();
    let (enlace, bateria) = enlace;
    app.enlace = enlace;
    app.bateria = bateria;
    app.hay_wallust = hay_programa("wallust");
    app.hay_look = color_del_look().is_ok();
    app.hay_cava = hay_programa("cava");
    let mut look = Look::load("chispa");
    let mut term = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    let r = (|| -> Result<(), String> {
        while !app.salir {
            look.refresh();
            term.draw(|f| dibujar(f, &app, &look)).map_err(|e| e.to_string())?;
            let ahora = Instant::now();
            if app.vencio(ahora) {
                app.escribir();
                continue;
            }
            if !event::poll(Duration::from_millis(100)).map_err(|e| e.to_string())? {
                continue;
            }
            let k = match event::read().map_err(|e| e.to_string())? {
                Event::Key(k) => k,
                Event::Mouse(m) => {
                    let s = term.size().map_err(|e| e.to_string())?;
                    if matches!(m.kind, MouseEventKind::Down(_)) {
                        app.aviso.clear();
                    }
                    if let Some(a) = app.raton(m, Rect::new(0, 0, s.width, s.height), Instant::now()) {
                        app.accion(a);
                    }
                    continue;
                }
                _ => continue,
            };
            if k.kind == KeyEventKind::Release {
                continue;
            }
            let ahora = Instant::now();
            app.aviso.clear();
            if let Some(a) = app.tecla(k, ahora) {
                app.accion(a);
            }
        }
        // lo que faltaba no se pierde al salir
        if app.pendiente.is_some() {
            app.escribir();
        }
        Ok(())
    })();
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    if app.mudo {
        return Err(app.aviso);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn bloque() -> Vec<u8> {
        let mut b = vec![0u8; 128];
        b[10] = 3;
        for i in 1..=34 {
            b[56 + 2 * i] = 9;
            b[57 + 2 * i] = 0x37;
        }
        b[126] = 0x5a;
        b[127] = 0xa5;
        b
    }

    fn app() -> App {
        App::new(bloque(), vec![0u8; LARGO_TABLA])
    }

    fn k(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    #[test]
    fn todas_las_teclas_existen_una_vez() {
        let mut vistas: Vec<&str> = FILAS.iter().flat_map(|f| f.iter().map(|k| k.1)).collect();
        vistas.sort();
        vistas.dedup();
        assert_eq!(vistas.len(), luz::TECLAS.len());
        assert!(vistas.iter().all(|n| luz::TECLAS.iter().any(|t| t.0 == *n)));
    }

    #[test]
    fn dejar_apretada_una_flecha_no_escribe_hasta_soltar() {
        let mut a = app();
        a.panel = Panel::Ajustes;
        let t0 = Instant::now();
        // 50 repeticiones, cada 30 ms: nunca vence
        for i in 0..50 {
            let t = t0 + Duration::from_millis(30 * i);
            assert!(a.tecla_normal(k(KeyCode::Left), t).is_none());
            assert!(!a.vencio(t));
        }
        let fin = t0 + Duration::from_millis(30 * 49);
        assert!(!a.vencio(fin + ESPERA - Duration::from_millis(1)));
        assert!(a.vencio(fin + ESPERA));
        // y es un solo cambio, sin pasarse del tope
        assert_eq!(luz::ajustes(&a.b_luz, 3).0, 0);
        assert!(luz::solo_luz(&a.luz, &a.b_luz));
        assert_eq!(a.luz, bloque(), "el bloque real no se toca antes de escribir");
    }

    #[test]
    fn la_l_en_ajustes_no_mueve_nada_sino_que_es_el_look() {
        let mut a = app();
        a.panel = Panel::Ajustes;
        a.tecla_normal(k(KeyCode::Char('h')), Instant::now());
        assert!(a.pendiente.is_none());
    }

    #[test]
    fn moverse_por_los_efectos_no_toca_nada() {
        let mut a = app();
        let t = Instant::now();
        a.tecla_normal(k(KeyCode::Down), t);
        a.tecla_normal(k(KeyCode::Down), t);
        assert!(a.pendiente.is_none());
        assert_eq!(a.b_luz, bloque());
        assert!(matches!(a.tecla_normal(k(KeyCode::Enter), t), Some(Accion::Efecto(_))));
    }

    #[test]
    fn pintar_pone_el_efecto_de_la_tabla() {
        let mut a = app();
        a.panel = Panel::Teclas;
        a.brocha = (1, 2, 3);
        let t = Instant::now();
        a.tecla_normal(k(KeyCode::Enter), t);
        assert_eq!(a.activo(), PERSONAL);
        assert_eq!(a.color_tecla(led("a")), (1, 2, 3));
        assert!(luz::solo_luz(&a.luz, &a.b_luz));
    }

    #[test]
    fn ajustes_que_el_efecto_no_tiene_no_salen_ni_cambian() {
        let mut a = app();
        // spectrum (3) no tiene color
        assert!(!a.visibles().contains(&COLOR));
        a.ajustar(COLOR, |_, _| 2, Instant::now());
        assert!(a.pendiente.is_none());
        assert_eq!(a.b_luz, bloque());
        // apagado solo queda la barrita
        luz::apagar(&mut a.b_luz);
        assert_eq!(a.visibles(), [LATERAL, SUENO]);
        assert_eq!(a.ajuste_visible(), LATERAL);
    }

    /// dibuja y regresa las zonas que quedaron
    fn dibujado(a: &App, w: u16, h: u16) -> Vec<(Rect, Clic)> {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        let look = Look::load("chispa");
        term.draw(|f| dibujar(f, a, &look)).unwrap();
        a.zonas.borrow().clone()
    }

    fn picar(a: &mut App, z: Clic, kind: MouseEventKind) -> Option<Accion> {
        let (r, _) = *a.zonas.borrow().iter().rev().find(|(_, c)| *c == z).expect("no se dibujó");
        let m = MouseEvent { kind, column: r.x, row: r.y, modifiers: KeyModifiers::NONE };
        a.raton(m, Rect::new(0, 0, 120, 30), Instant::now())
    }

    const CLIC: MouseEventKind = MouseEventKind::Down(MouseButton::Left);

    #[test]
    fn con_el_raton_se_escoge_efecto_ajuste_y_barrita() {
        let mut a = app();
        dibujado(&a, 120, 30);
        assert!(matches!(picar(&mut a, Clic::Efecto(0), CLIC), Some(Accion::Efecto(1))));
        // brillo al tope y la barrita apagada, en borrador
        dibujado(&a, 120, 30);
        picar(&mut a, Clic::Ajuste(BRILLO, 4), CLIC);
        picar(&mut a, Clic::Ajuste(LATERAL, 2), CLIC);
        assert_eq!(luz::ajustes(&a.b_luz, 3).0, 4);
        assert_eq!(luz::lateral(&a.b_luz), 2);
        assert!(a.pendiente.is_some());
        assert!(luz::solo_luz(&a.luz, &a.b_luz));
        // la rueda sube la barrita uno
        dibujado(&a, 120, 30);
        picar(&mut a, Clic::Fila(LATERAL), MouseEventKind::ScrollUp);
        assert_eq!(luz::lateral(&a.b_luz), 3);
    }

    #[test]
    fn dormir_camina_por_las_opciones_de_la_app() {
        let mut a = app();
        a.b_luz[luz::DORMIR] = 2;
        let t = Instant::now();
        a.ajustar(SUENO, |i, max| (i + 1).min(max), t);
        assert_eq!(luz::dormir(&a.b_luz), 3);
        a.ajustar(SUENO, |_, _| 0, t);
        assert_eq!(luz::dormir(&a.b_luz), 0, "nunca");
        a.ajustar(SUENO, |_, max| max, t);
        assert_eq!(luz::dormir(&a.b_luz), 40);
        assert!(luz::solo_luz(&a.luz, &a.b_luz));
    }

    #[test]
    fn la_ultima_celda_de_la_barra_es_el_tope() {
        let a = app();
        let z = dibujado(&a, 120, 30);
        let mut brillos: Vec<u8> = z.iter().filter_map(|(_, c)| if let Clic::Ajuste(BRILLO, v) = c { Some(*v) } else { None }).collect();
        assert_eq!(brillos.last(), Some(&9));
        assert!(brillos.windows(2).all(|w| w[0] <= w[1]));
        brillos.dedup();
        assert_eq!(brillos.len(), 10, "se llega a todos los valores");
    }

    #[test]
    fn clic_en_una_tecla_la_pinta_y_el_derecho_toma_su_color() {
        let mut a = app();
        a.brocha = (9, 8, 7);
        dibujado(&a, 120, 30);
        picar(&mut a, Clic::Tecla(3, 1), CLIC);
        assert_eq!(a.color_tecla(led("a")), (9, 8, 7));
        assert_eq!(a.activo(), PERSONAL);
        a.brocha = (0, 0, 0);
        dibujado(&a, 120, 30);
        picar(&mut a, Clic::Tecla(3, 1), MouseEventKind::Down(MouseButton::Right));
        assert_eq!(a.brocha, (9, 8, 7));
    }

    #[test]
    fn lo_que_no_se_puede_no_sale_en_los_botones() {
        let mut a = app();
        let hay = |a: &App, c: char| dibujado(a, 160, 30).iter().any(|(_, z)| *z == Clic::Boton(KeyCode::Char(c)));
        assert!(!hay(&a, 'w') && !hay(&a, 'l'));
        a.hay_wallust = true;
        a.hay_look = true;
        assert!(hay(&a, 'w') && hay(&a, 'l'));
        // los botones hacen lo mismo que su tecla
        dibujado(&a, 160, 30);
        picar(&mut a, Clic::Boton(KeyCode::Char('c')), CLIC);
        assert!(matches!(a.modo, Modo::Selector(_)));
        // fuera del selector, un clic lo cierra
        dibujado(&a, 160, 30);
        let m = MouseEvent { kind: CLIC, column: 0, row: 0, modifiers: KeyModifiers::NONE };
        a.raton(m, Rect::new(0, 0, 160, 30), Instant::now());
        assert!(a.modo == Modo::Normal);
    }

    #[test]
    fn con_el_selector_abierto_no_se_pinta_lo_de_abajo() {
        let mut a = app();
        a.modo = Modo::Selector(Selector::new((255, 0, 0)));
        let z = dibujado(&a, 120, 30);
        let caja = caja_selector(Rect::new(0, 0, 120, 30));
        let m = MouseEvent { kind: CLIC, column: caja.x + 2, row: caja.y + caja.height - 2, modifiers: KeyModifiers::NONE };
        assert!(z.iter().any(|(r, c)| *c == Clic::Selector && *r == caja));
        a.raton(m, Rect::new(0, 0, 120, 30), Instant::now());
        assert_eq!(a.b_tabla, vec![0u8; LARGO_TABLA]);
    }

    #[test]
    fn la_flecha_arriba_llega_a_la_tecla_mas_cercana() {
        let mut a = app();
        a.fila = 5;
        a.col = 3; // space
        a.mover_tecla(0, -1);
        assert_eq!(a.fila, 4);
        assert!(["c", "v", "b", "n", "m"].contains(&a.tecla_actual().1));
        a.fila = 0;
        a.col = 0;
        a.mover_tecla(0, -1);
        assert_eq!((a.fila, a.col), (0, 0));
    }

    #[test]
    fn el_selector_no_escribe_nada_hasta_pintar() {
        let mut a = app();
        let t = Instant::now();
        a.tecla_normal(k(KeyCode::Char('c')), t);
        assert!(matches!(a.modo, Modo::Selector(_)));
        a.tecla_selector(k(KeyCode::Right), t);
        a.tecla_selector(k(KeyCode::Down), t);
        a.tecla_selector(k(KeyCode::Right), t);
        assert!(a.pendiente.is_none());
        a.tecla_selector(k(KeyCode::Esc), t);
        assert!(a.modo == Modo::Normal);
        assert_eq!(a.brocha, (0x2a, 0x27, 0x40), "esc no cambia la brocha");
    }

    #[test]
    fn enter_en_el_selector_pinta_todo_el_teclado() {
        let mut a = app();
        let t = Instant::now();
        a.tecla_normal(k(KeyCode::Char('c')), t);
        if let Modo::Selector(s) = &mut a.modo {
            s.escoger(1);
        }
        a.tecla_selector(k(KeyCode::Enter), t);
        assert_eq!(a.brocha, (255, 80, 0));
        assert!(a.pendiente.is_some());
        assert_eq!(a.color_tecla(led("space")), (255, 80, 0));
        assert_eq!(a.activo(), PERSONAL);
    }

    #[test]
    fn desde_el_teclado_enter_solo_escoge_la_brocha() {
        let mut a = app();
        a.panel = Panel::Teclas;
        let t = Instant::now();
        a.tecla_normal(k(KeyCode::Char('c')), t);
        if let Modo::Selector(s) = &mut a.modo {
            s.escoger(1);
        }
        a.tecla_selector(k(KeyCode::Enter), t);
        assert_eq!(a.brocha, (255, 80, 0));
        assert!(a.pendiente.is_none());
        assert_eq!(a.color_tecla(led("space")), (0, 0, 0));
        // y luego enter pinta solo la tecla
        a.tecla_normal(k(KeyCode::Enter), t);
        assert_eq!(a.color_tecla(led("a")), (255, 80, 0));
        assert_eq!(a.color_tecla(led("space")), (0, 0, 0));
    }

    #[test]
    fn el_raton_mueve_las_barras_sin_salirse() {
        let mut a = app();
        a.modo = Modo::Selector(Selector::new((255, 0, 0)));
        let p = Rect::new(0, 0, 120, 30);
        let c = caja_selector(p);
        let clic = |column, row| MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column, row, modifiers: KeyModifiers::NONE };
        // clic en la muestra azul
        a.raton_selector(clic(c.x + 2 + 5 * 5, c.y + 1), p);
        let Modo::Selector(s) = &a.modo else { panic!() };
        assert_eq!(s.muestra, 5);
        // hasta el final de la barra de brillo, y más allá
        a.raton_selector(clic(c.x + c.width + 40, c.y + 1 + 5), p);
        let Modo::Selector(s) = &a.modo else { panic!() };
        assert_eq!((s.fila, s.v), (3, 1.0));
        // al principio: negro
        a.raton_selector(clic(c.x + 1 + ROTULO, c.y + 1 + 5), p);
        let Modo::Selector(s) = &a.modo else { panic!() };
        assert_eq!(s.v, 0.0);
    }

    #[test]
    fn el_selector_se_dibuja_en_ventanas_chicas() {
        for (w, h) in [(120, 30), (52, 11), (30, 8), (10, 4)] {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            let mut a = app();
            a.modo = Modo::Selector(Selector::new((255, 80, 0)));
            let look = Look::load("chispa");
            term.draw(|f| dibujar(f, &a, &look)).unwrap();
        }
    }

    #[test]
    fn el_color_mal_escrito_no_cambia_la_brocha() {
        let mut a = app();
        let antes = a.brocha;
        a.modo = Modo::Color("zzz".into());
        a.tecla_color(k(KeyCode::Enter));
        assert_eq!(a.brocha, antes);
        assert!(matches!(a.modo, Modo::Color(_)));
        a.modo = Modo::Color("#102030".into());
        a.tecla_color(k(KeyCode::Enter));
        assert_eq!(a.brocha, (0x10, 0x20, 0x30));
    }

    #[test]
    fn dibuja_sin_tronar_chico_y_grande() {
        for (w, h) in [(120, 30), (60, 14), (20, 5)] {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            let a = app();
            let look = Look::load("chispa");
            term.draw(|f| dibujar(f, &a, &look)).unwrap();
        }
    }
}
