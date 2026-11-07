# chispa

La luz del AULA F75 desde la terminal: efectos, brillo, color y tecla por
tecla, con el look de discada. Un binario: lo que se pone se queda guardado en
el teclado. Funciona por cable (`258a:010c`), por el dongle 2.4 GHz
(`3554:fa09`) y por Bluetooth (`3554:fa07`): encuentra solo por dónde está.

```
chispa                     cómo está la luz
chispa modes               los efectos
chispa mode wave --speed 3 --brightness 8 --color blue
chispa brightness 7
chispa speed 2
chispa color '#b9b6d8'     todo el teclado de un color
chispa key esc red w '#fff'
chispa look                con el color del look, oscurecido para que se vean las luces de Fn
chispa off / chispa on
chispa side breathing      la barrita blanca junto al Esc: off fill breathing static
chispa sleep 5             apagar las luces tras 5 min sin usarlo (never = nunca)
chispa battery             la batería, solo por 2.4 GHz (sin confirmar hasta verla bajar de 100)
chispa save                guardar ya el color en vivo (lo usa hypridle)
chispa restore             como estaba antes de usar chispa
chispa tui                 todo esto en una ventana, con el teclado dibujado
```

En la TUI todo se puede con el ratón: clic en un efecto lo pone, clic o
arrastrar en las barras las mueve, la rueda sube y baja, clic en una tecla la
pinta con la brocha y clic derecho toma su color. Lo que el efecto no tiene (o
lo que no se puede, como wallpaper sin wallust) no sale.

La barrita del Esc solo es blanca: no tiene color, solo modo.

## Sin parpadeo

El teclado apaga las luces un momento cada vez que guarda, por cable o por
radio, sea la tabla, los ajustes o las paletas: eso no se puede tapar. Así que
cuando cambian los colores de las teclas (wallpaper, `color`, `look`, o la TUI
pintando una o todas) no se guarda luego luego: se funde en vivo a lo nuevo y un
`chispa hold` de fondo lo sostiene (por radio, en 10 colores; al guardar, cada
tecla con el suyo). Lo guarda una vez y se sale:

- al bloquear la pantalla o antes de suspender (`chispa save`, desde hypridle),
- al salir de la sesión o apagar.

Mientras lo sostiene, el teclado se duerme a su hora (los cuadros no lo
despiertan) y al despertar sale directo el color nuevo. Si toca guardar y está
dormido, se guarda cuando despierte.

Todo esto es solo por 2.4 GHz o Bluetooth. **Por cable no hay nada en vivo**:
el teclado tarda ~67 ms en tomar cada cuadro, lo pinta barriendo de izquierda a
derecha, y mientras tanto no lee las teclas (con el video a 15 por segundo se
comía lo que tecleabas). Por cable los colores se guardan luego luego, con su
parpadeo; el video y el visualizador se pausan hasta que regrese la radio, y lo
sostenido se guarda en cuanto se conecta el cable.

`chispa` dice si hay un color en vivo sin guardar. Lo que hace lo de fondo (`hold`, `video`, el
gancho del wallpaper) queda en `~/.local/state/chispa/log`: qué pasó y los
errores, nunca lo que tecleas. Lo demás (efectos, brillo,
una tecla, la barrita, el sleep) se guarda como siempre: primero el de fondo
guarda lo suyo. Apagar se funde a negro y guarda a oscuras, sin que se vea;
prender sube desde negro y parpadea al guardar.

Las comillas en `'#b9b6d8'` son para que la shell no lo tome como comentario.

## Permisos

```
sudo install -m644 60-aula-f75.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger
```

## Cuidados

- Solo manda comandos de luz: por cable el reporte 6, por radio el 0x13. Nunca el
  reporte 5 (puede meter al teclado a su bootloader), ni firmware, reset o teclas.
  Ojo: los números no se comparten. El 0x06 que por cable escribe la tabla, por
  radio es el reset de fábrica; por eso cada enlace trae su lista cerrada.
- Por radio espera a que sueltes las teclas antes de hablarle: si no, se puede
  perder cuando sueltas una y la compu la repite sin parar.
- Edita el bloque que regresa el teclado, nunca arma uno nuevo: ahí mismo viven
  el modo Mac y el polling. Se niega a escribir si cambiaría algo que no sea
  la luz, la barrita o el sleep (y esos dos, solo con valores que existen).
- No escribe si nada cambió, porque esto se guarda en la memoria del teclado.
- Vuelve a leer lo que escribió; si no quedó igual, regresa como estaba.
- Antes de la primera escritura respalda los bloques en
  `~/.local/state/chispa/respaldo.bin`.

El protocolo sale de lo que ya documentaron
[aula-f75-linux](https://github.com/daschinmoy21/aula-f75-linux),
[aula-f75-hub](https://github.com/bakiakyol/aula-f75-hub),
[aulaf75-nativehub](https://github.com/kveld9/aulaf75-nativehub) y
[aula-f75-webdriver](https://github.com/vinicenter/aula-f75-webdriver),
revisado contra un F75 de verdad.

## Seguir al wallpaper

`chispa wallpaper` pinta el teclado con el wallpaper, tecla por tecla: cada una
con el pedazo de imagen que le queda encima, como una pantallita. Y se queda
siguiéndolo (`KEYBOARD_LIGHT=wallpaper` en `hypr/discada.conf`): cada wallpaper
nuevo lo avisa `WallustSwww.sh` con `chispa wallpaper --auto`. Escoger un color o
un efecto a mano deja de seguirlo.

Con un wallpaper en video (mpvpaper), `chispa video` lo sigue en vivo (solo
por radio, ~15 cuadros por segundo: es el techo medido, 9 paquetes de ~7 ms).
Cada video se achica una vez con ffmpeg por la GPU (48×18, ~2.3 MB por minuto)
en `~/.cache/chispa/video`; de ahí solo se lee el cuadro que toca según dónde
va mpvpaper, que se le pregunta por su socket (`input-ipc-server`, que pone
`WallpaperSelect.sh`). No se queda nada decodificando. Lo lanza solo el gancho
del wallpaper y el arranque de la sesión.

## Visualizador

`chispa music` pone barras con el audio: graves a la izquierda, agudos a la
derecha, en tonos alrededor del color del wallpaper. Son cuadros en vivo, así que
no se escribe nada en la memoria del teclado. Sin música no manda nada y el
teclado regresa solo a su luz. Necesita `cava`. Escoger un color o efecto a mano,
`chispa music stop` o Ctrl+C lo apagan. Con `KEYBOARD_LIGHT=music` arranca solo
al entrar a la sesión (`chispa music --auto`).
