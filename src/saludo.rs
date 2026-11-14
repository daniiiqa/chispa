//! `chispa saludo`: la entrada de discada en el teclado. Un barrido de luz de
//! izquierda a derecha con el color del look (la punta casi blanca y una
//! estela que se apaga), todo el teclado se enciende y se funde en la luz
//! que ya tenía. Son cuadros en vivo: no se guarda nada (por cable no hay
//! cuadros en vivo, ver Teclado::en_vivo: ahí no hace nada).

use crate::hid::Teclado;
use crate::luz::{self, Rgb};
use std::thread::sleep;
use std::time::Duration;

const COLUMNAS: usize = 15;
const FILAS: usize = 6;
/// lo que aguanta la radio entre cuadros (ver musica::ENTRE_CUADROS)
const PASO: Duration = Duration::from_millis(66);
/// la estela: cuánto brilla cada columna detrás de la punta
const ESTELA: [f32; 5] = [1.0, 0.7, 0.45, 0.25, 0.1];

fn escalar((r, g, b): Rgb, f: f32) -> Rgb {
    let m = |x: u8| (x as f32 * f).round() as u8;
    (m(r), m(g), m(b))
}

pub fn correr() -> Result<(), String> {
    let c = crate::color_del_look().unwrap_or((150, 120, 255));
    let punta = luz::mezclar(c, (255, 255, 255), 0.6);
    let t = Teclado::abrir()?;
    if !t.en_vivo() {
        return Ok(());
    }
    let leds: Vec<u8> = luz::TECLAS.iter().map(|k| k.1 as u8).collect();
    let columna = |col: usize| -> Vec<u8> { leds.iter().copied().filter(|l| *l as usize / FILAS == col).collect() };

    // 1) el barrido: una columna por cuadro, con su estela
    for paso in 0..COLUMNAS + ESTELA.len() {
        let mut grupos: Vec<(Rgb, Vec<u8>)> = Vec::new();
        for (d, f) in ESTELA.iter().enumerate() {
            let Some(col) = paso.checked_sub(d).filter(|c| *c < COLUMNAS) else { continue };
            let teclas = columna(col);
            if !teclas.is_empty() {
                grupos.push((if d == 0 { punta } else { escalar(c, *f) }, teclas));
            }
        }
        t.cuadro(&grupos)?;
        sleep(PASO);
    }

    // 2) todo el teclado se prende y se funde en lo que tenía (si se sabe; si
    //    no, se apaga suave y a los 2 s el teclado regresa solo a su luz)
    let todo = luz::mapa_uniforme(c);
    let antes = t.visto().unwrap_or_else(|| luz::mapa_uniforme((0, 0, 0)));
    t.cuadro(&luz::grupos(&todo, true))?;
    sleep(PASO * 2);
    for k in 1..=10 {
        let m = luz::mezclar_mapa(&todo, &antes, k as f32 / 10.0);
        t.cuadro(&luz::grupos(&m, true))?;
        sleep(PASO);
    }
    Ok(())
}
