# chispa

La luz del AULA F75 Pro desde la terminal: efectos, brillo, color y tecla por
tecla, con el look de discada. Sigue al wallpaper (también en video) y tiene
visualizador. Funciona por cable (`258a:010c`), por el dongle 2.4 GHz
(`3554:fa09`) y por Bluetooth (`3554:fa07`): encuentra solo por dónde está.

## Usar

```
chispa                     cómo está la luz (y si hay algo en vivo sin guardar)
chispa modes               los efectos
chispa mode wave --speed 3 --brightness 8 --color blue
chispa brightness 7
chispa speed 2
chispa color '#b9b6d8'     todo el teclado de un color
chispa key esc red w '#fff'
chispa look                con el color del look, oscurecido para que se vean las luces de Fn
chispa wallpaper           el teclado sigue al wallpaper, tecla por tecla
chispa music               el visualizador (Ctrl+C o `music stop`)
chispa video               seguir el wallpaper en video (lo lanza solo el gancho)
chispa off / chispa on
chispa side breathing      la barrita blanca junto al Esc: off fill breathing static
chispa sleep 5             apagar las luces tras 5 min sin usarlo (never = nunca)
chispa knob lighting       la perilla: volume (volumen y silencio) o lighting (brillo y efecto)
chispa battery [--cached]  la batería, solo por 2.4 GHz; --cached: la última leída, al instante
chispa save                guardar ya lo que está en vivo (lo usa hypridle)
chispa restore             como estaba antes de usar chispa
chispa tui                 todo esto en una ventana, con el teclado dibujado
```

Las comillas en `'#b9b6d8'` son para que la shell no lo tome como comentario.
La barrita del Esc solo es blanca: no tiene color, solo modo. La batería sale
con "(?)" hasta verla bajar de 100: el número viene de un comando que dos
proyectos leen como batería y otro como "perfil", y falta confirmarlo.

En la TUI (SUPER+Shift+L) todo se puede con el ratón: clic en un efecto lo pone,
clic o arrastrar en las barras las mueve, la rueda sube y baja, clic en una
tecla la pinta con la brocha y clic derecho toma su color. Con video, el teclado
dibujado se mueve con él. Lo que no se puede (wallpaper sin wallust, música sin
cava, lo que el efecto no tiene) no sale.

## Seguir al wallpaper

Con `KEYBOARD_LIGHT=wallpaper` en `hypr/discada.conf`, cada wallpaper nuevo lo
avisa `WallustSwww.sh` (`chispa wallpaper --auto`) y el teclado lo pinta tecla
por tecla: cada una con el pedazo de imagen que le queda encima, como una
pantallita. Escoger un color o un efecto a mano deja de seguirlo.

Con un wallpaper en video (mpvpaper), `chispa video` lo sigue en vivo a ~15
cuadros por segundo. Cada video se achica una vez con ffmpeg por la GPU (48×18,
~2.3 MB por minuto) en `~/.cache/chispa/video`, y de ahí solo se lee el cuadro
que toca según dónde va mpvpaper, que se le pregunta por su socket
(`input-ipc-server`, que pone `WallpaperSelect.sh`). No se queda nada
decodificando. Lo lanzan el gancho del wallpaper y el arranque de la sesión.

## Visualizador

`chispa music` pone barras con el audio: graves a la izquierda, agudos a la
derecha, en tonos alrededor del color del wallpaper. Son cuadros en vivo: no
se escribe nada en la memoria del teclado. Sin música no manda nada y el
teclado regresa solo a su luz. Necesita `cava`. Con `KEYBOARD_LIGHT=music`
arranca solo al entrar a la sesión (`chispa music --auto`).

## Sin parpadeo

El teclado apaga las luces un momento **cada vez que guarda**, por cable o por
radio, sea la tabla, los ajustes o las paletas, y eso no se puede tapar. Así
que cuando cambian los colores de las teclas (wallpaper, `color`, `look`, la TUI
pintando una o todas) no se guarda luego luego: se funde en vivo a lo nuevo y
un `chispa hold` de fondo lo sostiene (por radio, en 10 colores; al guardar,
cada tecla con el suyo). Lo guarda una vez y se sale:

- al bloquear la pantalla o antes de suspender (`chispa save`, desde hypridle),
- al salir de la sesión o apagar.

Mientras lo sostiene, el teclado se duerme a su hora y al despertar sale
directo el color nuevo. Si toca guardar y está dormido, se guarda cuando
despierte. Lo demás (efectos, brillo, una tecla, la barrita, el sleep) se
guarda como siempre: primero el de fondo guarda lo suyo. Apagar se funde a
negro y guarda a oscuras, sin que se vea; prender sube desde negro.

## Cable o radio

Lo en vivo (sostener, video, visualizador) **solo es por 2.4 GHz o
Bluetooth**. Por cable el teclado tarda 66 ms en tomar cada cuadro, lo pinta
barriendo de izquierda a derecha y mientras tanto no lee las teclas: con el
video se comía lo que tecleabas. No hay forma de hacerlo más rápido (ver
abajo). Por cable los colores se guardan luego luego, con su parpadeo; el video
y el visualizador se pausan hasta que regrese la radio, y lo sostenido se
guarda en cuanto se conecta el cable.

## Lo medido en el teclado

| qué | cuánto |
|---|---|
| un cuadro en vivo por radio | 9 paquetes de ~7 ms = 63 ms → techo de ~15 por segundo |
| un cuadro en vivo por cable | 66 ms, sin importar cuántos bytes: el reporte 6 siempre mide 520 y el endpoint de control es de 8 bytes (65 paquetes) |
| cuadros por radio | caben 10 colores (10 × 4 + 80 teclas = 120 de 126 bytes) |
| sin cuadros | regresa a lo guardado a los ~2 s; si se atrasan (tecleando), pinta todo del primer color: por eso el más visto va primero |
| guardar | siempre apaga las luces un momento; cuadros a media escritura la echan a perder |
| dormido | los cuadros no lo despiertan, la perilla tampoco |

## Si algo falla

Lo de fondo (`hold`, `video`, `music`, el gancho del wallpaper) anota qué pasó y
sus errores en `~/.local/state/chispa/log`, nunca lo que tecleas. Se reinicia
solo cuando se instala un chispa nuevo.

## Instalar

```
cargo build --release
install -m755 target/release/chispa ~/.local/bin/
sudo install -m644 60-aula-f75.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger
```

Para el video hace falta `ffmpeg` (y VAAPI para que sea por la GPU); para el
visualizador, `cava`; para seguir al wallpaper, `wallust`.

## Cuidados

- Solo manda comandos de luz: por cable el reporte 6, por radio el 0x13. Nunca el
  reporte 5 (puede meter al teclado a su bootloader), ni firmware, reset o teclas.
  Ojo: los números no se comparten. El 0x06 que por cable escribe la tabla, por
  radio es el reset de fábrica; por eso cada enlace trae su lista cerrada.
- Por radio espera a que sueltes las teclas antes de hablarle: si no, se puede
  perder cuando sueltas una y la compu la repite sin parar.
- Edita el bloque que regresa el teclado, nunca arma uno nuevo: ahí mismo viven
  el modo Mac y el polling. Se niega a escribir si cambiaría algo que no sea la
  luz, la barrita, el sleep o la perilla (y esos, solo con valores que existen).
- Guarda encima de lo que el teclado tiene en ese momento: lo que no quiso
  cambiar se queda como está (si apretaste la perilla mientras tanto, no la
  regresa).
- No escribe si nada cambió, porque esto se guarda en la memoria del teclado.
- Vuelve a leer lo que escribió; si no quedó igual, regresa como estaba.
- Antes de la primera escritura respalda los bloques en
  `~/.local/state/chispa/respaldo.bin`.

## De dónde sale

El protocolo sale de lo que ya documentaron
[aula-f75-linux](https://github.com/daschinmoy21/aula-f75-linux),
[aula-f75-hub](https://github.com/bakiakyol/aula-f75-hub),
[aulaf75-nativehub](https://github.com/kveld9/aulaf75-nativehub),
[aula-f75-webdriver](https://github.com/vinicenter/aula-f75-webdriver),
[f75-studio](https://github.com/trenbolone1122/aula-f75-linux) y
[Aula Control](https://github.com/FaizAsta/Aula-F75-software), revisado y medido
contra un F75 Pro de verdad.
