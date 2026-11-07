//! Los paquetes del teclado por radio (2.4 GHz y Bluetooth), sin tocar el
//! teclado: reporte 0x13, 20 bytes con suma al final. Por cable es otro
//! protocolo, y sus números no se pueden reusar aquí: el 0x06 que por cable
//! escribe la tabla, por radio es el RESET DE FÁBRICA.

pub const REPORTE: u8 = 0x13;
pub const LARGO: usize = 20;
const TROZO: usize = 14;

/// Lo único que chispa le manda por radio: leer y escribir los dos bloques de la luz.
pub const LEER_LUZ: u8 = 0x44;
pub const ESCRIBIR_LUZ: u8 = 0x04;
pub const LEER_TABLA: u8 = 0x42;
pub const ESCRIBIR_TABLA: u8 = 0x02;
/// Un cuadro en vivo: no se guarda, y el teclado no contesta.
pub const VIVO: u8 = 0x88;
const VIVO_GRUPOS: u8 = 1;
// Probado y descartado (2026-11-07): guardar la paleta del estático (0x09) apaga
// las luces igual que la tabla. Por radio, todo lo que se guarda las apaga un rato.
/// Solo se lee. ¿La batería? aula-f75-hub y Aula Control (FaizAsta) leen aquí el
/// %, pero lo que se sacó del driver de AULA le dice "perfil", y la app oficial
/// ni enseña batería (KB.ini: ShowPower=0). Daba 100 con Fn+B en 10/10: falta
/// verlo bajar. Por cable no existe.
pub const LEER_BATERIA: u8 = 0x4a;
const PERMITIDOS: [u8; 6] = [LEER_LUZ, ESCRIBIR_LUZ, LEER_TABLA, ESCRIBIR_TABLA, VIVO, LEER_BATERIA];

/// La última revisión antes de que un paquete salga.
pub fn permitido(p: &[u8]) -> bool {
    p.len() == LARGO && p[0] == REPORTE && PERMITIDOS.contains(&p[1])
}

fn suma(p: &[u8]) -> u8 {
    p[..LARGO - 1].iter().fold(0u8, |s, x| s.wrapping_add(*x))
}

fn paquete(cmd: u8, sub: u8, total: u8, indice: u8, trozo: &[u8]) -> [u8; LARGO] {
    let mut p = [0u8; LARGO];
    p[0] = REPORTE;
    p[1] = cmd;
    p[2] = total;
    p[3] = indice;
    p[4] = (sub << 4) | trozo.len() as u8;
    p[5..5 + trozo.len()].copy_from_slice(trozo);
    p[LARGO - 1] = suma(&p);
    p
}

pub fn pedir(cmd: u8) -> [u8; LARGO] {
    paquete(cmd, 0, 1, 0, &[])
}

pub fn partir(cmd: u8, datos: &[u8]) -> Vec<[u8; LARGO]> {
    let total = datos.len().div_ceil(TROZO) as u8;
    datos.chunks(TROZO).enumerate().map(|(i, t)| paquete(cmd, 0, total, i as u8, t)).collect()
}

/// Los paquetes de un cuadro en vivo, con colores por grupos.
pub fn cuadro(datos: &[u8]) -> Vec<[u8; LARGO]> {
    let total = datos.len().div_ceil(TROZO) as u8;
    datos.chunks(TROZO).enumerate().map(|(i, t)| paquete(VIVO, VIVO_GRUPOS, total, i as u8, t)).collect()
}

#[derive(Debug, PartialEq)]
pub struct Respuesta<'a> {
    pub cmd: u8,
    pub total: usize,
    pub indice: usize,
    pub error: bool,
    pub datos: &'a [u8],
}

/// None si no es uno de los nuestros: por la misma interfaz llega el teclado normal.
pub fn abrir(p: &[u8]) -> Option<Respuesta<'_>> {
    if p.len() < LARGO || p[0] != REPORTE || suma(p) != p[LARGO - 1] {
        return None;
    }
    let n = (p[4] & 0x0f) as usize;
    if n > TROZO {
        return None;
    }
    Some(Respuesta { cmd: p[1], total: (p[2] & 0x7f) as usize, indice: p[3] as usize, error: p[2] & 0x80 != 0, datos: &p[5..5 + n] })
}

/// Junta las partes de una respuesta por su índice. Por radio se pierden
/// paquetes y el teclado siempre manda todo otra vez, así que sobrevive entre
/// intentos.
#[derive(Default)]
pub struct Armado {
    partes: Vec<Option<Vec<u8>>>,
}

impl Armado {
    pub fn agregar(&mut self, r: &Respuesta) {
        if r.total == 0 || r.indice >= r.total {
            return;
        }
        if self.partes.len() != r.total {
            self.partes = vec![None; r.total];
        }
        self.partes[r.indice] = Some(r.datos.to_vec());
    }

    pub fn vacio(&self) -> bool {
        self.partes.is_empty()
    }

    pub fn completo(&self) -> Option<Vec<u8>> {
        if self.vacio() {
            return None;
        }
        self.partes.iter().try_fold(Vec::new(), |mut v, p| {
            v.extend_from_slice(p.as_ref()?);
            Some(v)
        })
    }
}

/// ¿Hay una tecla abajo según este reporte? None si no es de teclas. Si se le
/// habla mientras hay una tecla abajo se puede perder cuando la sueltas, y la
/// compu la repite sin parar.
pub fn teclas_abajo(r: &[u8], bluetooth: bool) -> Option<bool> {
    // en los reportes de 6 teclas, 1-3 no son teclas sino errores: el teclado
    // manda "ErrorRollOver" (1) cada vez que se suelta una
    let hay = |t: &[u8]| t.iter().any(|k| *k > 3);
    let primero = *r.first()?;
    if bluetooth {
        return match primero {
            1 if r.len() >= 8 => Some(r[1] != 0 || hay(&r[3..8])),
            2 => Some(r[1..].iter().any(|b| *b != 0)),
            _ => None,
        };
    }
    match r.len() {
        8 => Some(primero != 0 || hay(&r[2..7])),
        n if n > 8 && primero == 4 => Some(r[1..].iter().any(|b| *b != 0)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_paquete_trae_su_forma_y_su_suma() {
        // el mismo pedido que manda la app oficial para leer los ajustes
        let p = pedir(LEER_LUZ);
        assert_eq!(&p[..5], &[0x13, 0x44, 1, 0, 0]);
        assert_eq!(p[19], (0x13u8 + 0x44 + 1));
        assert!(abrir(&p).is_some());
        let mut mala = p;
        mala[7] ^= 1;
        assert!(abrir(&mala).is_none(), "la suma tiene que notar el cambio");
    }

    #[test]
    fn nunca_sale_un_comando_peligroso() {
        for cmd in [0x01, 0x03, 0x06, 0x07, 0x09, 0x0a, 0x0b, 0x41, 0x43, 0x8e] {
            assert!(!permitido(&pedir(cmd)), "{cmd:#x} no debe salir");
        }
        for cmd in PERMITIDOS {
            assert!(permitido(&pedir(cmd)));
        }
        let mut otro = pedir(LEER_LUZ);
        otro[0] = 6;
        assert!(!permitido(&otro), "ni otro reporte");
    }

    #[test]
    fn los_bloques_se_parten_en_trozos_de_14() {
        let datos: Vec<u8> = (0..128).map(|i| i as u8).collect();
        let ps = partir(ESCRIBIR_LUZ, &datos);
        assert_eq!(ps.len(), 10);
        assert_eq!(ps[0][2], 10);
        assert_eq!(ps[9][3], 9);
        assert_eq!(ps[9][4], 2, "128 = 9×14 + 2");
        assert_eq!(partir(ESCRIBIR_TABLA, &[0; 378]).len(), 27);
        let mut a = Armado::default();
        for p in &ps {
            a.agregar(&abrir(p).unwrap());
        }
        assert_eq!(a.completo().unwrap(), datos);
    }

    #[test]
    fn un_paquete_perdido_se_completa_en_otro_intento() {
        let datos = [7u8; 40];
        let ps = partir(LEER_LUZ, &datos);
        let mut a = Armado::default();
        a.agregar(&abrir(&ps[0]).unwrap());
        a.agregar(&abrir(&ps[2]).unwrap());
        assert!(a.completo().is_none(), "con un hueco no se regresa nada");
        a.agregar(&abrir(&ps[1]).unwrap());
        assert_eq!(a.completo().unwrap(), datos);
    }

    #[test]
    fn un_cuadro_lleva_su_sub_y_cabe_en_9_paquetes() {
        let ps = cuadro(&[9u8; 126]);
        assert_eq!(ps.len(), 9);
        assert_eq!(&ps[0][..5], &[0x13, 0x88, 9, 0, 0x1e]);
        assert!(ps.iter().all(|p| permitido(p)));
    }

    #[test]
    fn un_error_del_teclado_se_nota() {
        let mut p = pedir(LEER_LUZ);
        p[2] |= 0x80;
        p[19] = suma(&p);
        assert!(abrir(&p).unwrap().error);
    }

    #[test]
    fn se_ven_las_teclas_abajo() {
        // 2.4 GHz: [mods, 0, 5 teclas, vendor]
        assert_eq!(teclas_abajo(&[0, 0, 0x04, 0, 0, 0, 0, 0], false), Some(true));
        assert_eq!(teclas_abajo(&[0x08, 0, 0, 0, 0, 0, 0, 0], false), Some(true));
        assert_eq!(teclas_abajo(&[0; 8], false), Some(false));
        assert_eq!(teclas_abajo(&[0, 0, 1, 0, 0, 0, 0, 0], false), Some(false), "ErrorRollOver no es una tecla");
        let mut nkro = [0u8; 21];
        nkro[0] = 4;
        assert_eq!(teclas_abajo(&nkro, false), Some(false));
        nkro[5] = 2;
        assert_eq!(teclas_abajo(&nkro, false), Some(true));
        assert_eq!(teclas_abajo(&pedir(LEER_LUZ), false), None, "la respuesta de la luz no es de teclas");
        // Bluetooth: todo llega por un nodo, y el 4 ahí es otra cosa
        assert_eq!(teclas_abajo(&[1, 0, 0, 0x04, 0, 0, 0, 0], true), Some(true));
        assert_eq!(teclas_abajo(&[1, 0, 0, 0, 0, 0, 0, 0], true), Some(false));
        assert_eq!(teclas_abajo(&[2, 0, 1], true), Some(true));
        assert_eq!(teclas_abajo(&[4, 1], true), None);
    }
}
