// Textos en inglés. La clave es el texto en español tal como está en el código.
const dict: Record<string, string> = {
  // Ajustes: idioma
  "Idioma": "Language",
  "Idioma de la interfaz. Automático usa el de Windows.": "Interface language. Automatic follows Windows.",
  "Automático": "Automatic",

  // Ajustes: cabecera y avisos
  "Ajustes": "Settings",
  "Los cambios de grabación se aplican desde la próxima partida.": "Recording changes apply from the next match.",
  "Guardar cambios": "Save changes",
  "No se pudieron registrar estos atajos (¿los usa otra app?): {list}": "Couldn't register these hotkeys (is another app using them?): {list}",
  "Ajustes guardados": "Settings saved",
  "Los clips que siguen a la calidad de grabación pasan a 1080p liviano y se reemplazan. La partida completa no se toca. ¿Comprimirlos?":
    "Clips still at recording quality are converted to lightweight 1080p and replaced. The full match is left untouched. Compress them?",
  "{n} clips comprimidos · {size} liberados": "{n} clips compressed · {size} freed",
  "No había clips por comprimir": "There were no clips to compress",

  // Ajustes: estado del sistema
  "Estado del sistema": "System status",
  "ffmpeg no encontrado ({path})": "ffmpeg not found ({path})",
  "Codificador {name} disponible": "{name} encoder available",
  "{name} no funciona en esta PC: se graba con {used}": "{name} doesn't work on this PC: recording with {used}",
  "No se encontró ningún codificador de video que funcione": "No working video encoder was found",
  "Log de Fortnite encontrado": "Fortnite log found",
  "No se encontró el log de Fortnite (¿está instalado?)": "Fortnite log not found (is it installed?)",
  "Las repeticiones están desactivadas en Fortnite: actívalas (Ajustes → Juego → Repeticiones) para detectar eliminaciones":
    "Replays are turned off in Fortnite: turn them on (Settings → Game → Replays) to detect eliminations",
  "Repeticiones de Fortnite activas (se usan para detectar eliminaciones)": "Fortnite replays are on (used to detect eliminations)",

  // Codificadores
  "NVIDIA H.264 (recomendado)": "NVIDIA H.264 (recommended)",
  "AMD H.264 (recomendado)": "AMD H.264 (recommended)",
  "Intel H.264 (recomendado)": "Intel H.264 (recommended)",
  "Por software · H.264 (usa el procesador)": "Software · H.264 (uses the CPU)",
  "{name} (no disponible)": "{name} (unavailable)",

  // Ajustes: grabación
  "Grabación": "Recording",
  "Grabar partidas automáticamente": "Record matches automatically",
  "Empezar a grabar": "Start recording",
  "La isla previa incluye el calentamiento": "The pre-game island includes the warm-up",
  "Al empezar la partida (isla previa)": "When the match starts (pre-game island)",
  "Al subir al autobús": "When boarding the bus",
  "Terminar de grabar": "Stop recording",
  "Cuando te eliminan o ganas": "When you're eliminated or win",
  "Al volver al lobby (incluye espectador)": "When back in the lobby (includes spectating)",
  "Segundos extra al final": "Extra seconds at the end",
  "Automático usa la pantalla donde está Fortnite": "Automatic uses the screen Fortnite is on",
  "Pantalla {n} · {w}×{h}": "Screen {n} · {w}×{h}",
  "Codificador": "Encoder",
  "H.264 se reproduce en todos lados; HEVC/AV1 pesan menos. Se listan los que funcionan con tu tarjeta gráfica; por software sirve en cualquier PC pero usa mucho procesador.":
    "H.264 plays everywhere; HEVC/AV1 make smaller files. Only the ones that work with your graphics card are listed; software works on any PC but uses a lot of CPU.",
  "Calidad": "Quality",
  "Menor número = más calidad y archivos más grandes": "Lower number = higher quality and bigger files",
  "Máxima (CQ 16)": "Maximum (CQ 16)",
  "Alta (CQ 19)": "High (CQ 19)",
  "Media (CQ 23)": "Medium (CQ 23)",
  "Ligera (CQ 27)": "Light (CQ 27)",
  "Cuadros por segundo": "Frames per second",
  "Bitrate máximo (Mbps)": "Max bitrate (Mbps)",

  // Ajustes: otros juegos
  "Otros juegos": "Other games",
  "Además de Fortnite, ggswp graba la sesión entera de estos juegos mientras estén abiertos. No detecta eliminaciones ni arma clips solo: usa los atajos para marcar momentos y guardar clips.":
    "Besides Fortnite, ggswp records the whole session of these games while they are open. It doesn't detect eliminations or make clips on its own: use the hotkeys to bookmark moments and save clips.",
  "Nombre de {exe}": "Name for {exe}",
  "Quitar": "Remove",
  "Agregar un juego": "Add a game",
  "Abre el juego y elígelo de la lista de programas abiertos.": "Open the game and pick it from the list of running programs.",
  "Elegir un programa abierto…": "Pick a running program…",
  "Buscar programas abiertos…": "Look for running programs…",

  // Ajustes: audio
  "Audio del juego y del sistema": "Game and system audio",
  "Separar el audio del juego": "Separate game audio",
  "Graba el juego en una pista y el resto (Discord, música, navegador) en otra, para elegir qué lleva cada clip. Necesita Windows 10 de 2020 o más nuevo; si no se puede, graba todo junto.":
    "Records the game on one track and everything else (Discord, music, browser) on another, so you can choose what goes into each clip. Needs Windows 10 from 2020 or newer; if it's not possible, everything is recorded together.",
  "Los clips incluyen las otras apps": "Clips include other apps",
  "Apagado, los clips llevan solo el sonido del juego. La partida completa siempre guarda todo, y en el editor puedes cambiarlo clip por clip.":
    "When off, clips carry only the game sound. The full match always keeps everything, and you can change it clip by clip in the editor.",
  "Micrófono": "Microphone",
  "Volumen del micrófono": "Microphone volume",
  "Los clips incluyen el micrófono": "Clips include the microphone",

  // Ajustes: clips
  "Formato de los clips": "Clip format",
  "Para compartir: H.264 a 1080p, cerca de 1 MB por segundo; entra en WhatsApp e Instagram. Original: como la grabación, unas 10 veces más pesado.":
    "For sharing: H.264 at 1080p, about 1 MB per second; fits WhatsApp and Instagram. Original: same as the recording, about 10 times bigger.",
  "Para compartir · 1080p (recomendado)": "For sharing · 1080p (recommended)",
  "Calidad original": "Original quality",
  "Clips que ya tienes": "Clips you already have",
  "Pasa al formato liviano los clips guardados a la calidad de grabación. El avance se ve en cada partida.":
    "Converts clips saved at recording quality to the lightweight format. Progress is shown on each match.",
  "Comprimiendo…": "Compressing…",
  "Comprimir clips existentes": "Compress existing clips",
  "Contar cofres abiertos": "Count opened chests",
  "Al terminar la partida lee en el video los avisos de experiencia (+PE · COFRE). Tarda unos segundos por cada minuto de partida.":
    "When the match ends, it reads the XP pop-ups in the video (+XP · CHEST). Takes a few seconds per minute of match.",
  "Crear clips de eliminaciones y victorias": "Create clips of eliminations and wins",
  "Mejor jugada lista para subir": "Best play ready to post",
  "Al terminar cada partida elige su mejor clip y deja una versión 9:16 para TikTok, Reels y Shorts.":
    "After each match, picks its best clip and makes a 9:16 version for TikTok, Reels and Shorts.",
  "Modo móvil (con salud y armas)": "Mobile mode (with health and weapons)",
  "No crearla": "Don't create it",
  "Segundos antes del momento": "Seconds before the moment",
  "Segundos después": "Seconds after",
  "Incluir el momento en que te eliminan": "Include the moment you get eliminated",

  // Ajustes: atajos y highlights
  "Atajos y highlights": "Hotkeys and highlights",
  "Guardar los últimos segundos": "Save the last seconds",
  "Funciona dentro y fuera de partida": "Works in and out of a match",
  "Duración del clip": "Clip length",
  "Últimos {n} s": "Last {n} s",
  "Marcar momento": "Bookmark moment",
  "Agrega una marca en la línea de tiempo de la partida": "Adds a bookmark to the match timeline",
  "Iniciar / detener grabación": "Start / stop recording",
  "Menú de highlights sobre el juego": "In-game highlights menu",
  "Búfer de repetición": "Replay buffer",
  "Mientras Fortnite está abierto graba en segundo plano (temporal) para poder guardar clips también en el lobby":
    "While Fortnite is open, it records in the background (temporary) so you can save clips in the lobby too",
  "Avisos sobre el juego": "In-game notifications",
  "Requiere Fortnite en modo Ventana sin bordes": "Requires Fortnite in Borderless Window mode",
  "Avisar cuando se registra un momento destacado": "Notify when a highlight is logged",

  // Ajustes: biblioteca
  "Biblioteca": "Library",
  "Carpeta de grabaciones": "Recordings folder",
  "Cambiar…": "Change…",
  "Borrar de la PC lo que ya está en Drive": "Delete from the PC what is already on Drive",
  "Antes de borrar se comprueba que la copia de Drive esté completa. Lo que no se subió se queda en la PC. Puedes volver a descargarlo desde la partida.":
    "Before deleting, the Drive copy is checked to be complete. Anything not uploaded stays on the PC. You can download it again from the match.",
  "Apenas se sube": "As soon as it uploads",
  "A los 3 días": "After 3 days",
  "A los 7 días": "After 7 days",
  "A los 14 días": "After 14 days",
  "Nunca": "Never",
  "Borrar del todo (PC y Drive) después de": "Delete completely (PC and Drive) after",
  "Solo las partidas no conservadas. En Drive van a la papelera, donde se pueden recuperar durante 30 días.":
    "Only matches you haven't kept. On Drive they go to the trash, where they can be recovered for 30 days.",
  "7 días": "7 days",
  "15 días": "15 days",
  "30 días": "30 days",
  "60 días": "60 days",

  // Ajustes: Google Drive
  "Cuenta conectada": "Connected account",
  "Conectada": "Connected",
  "Desconectar": "Disconnect",
  "Subir automáticamente al terminar cada partida": "Upload automatically after each match",
  "Subir clips": "Upload clips",
  "Subir la partida completa": "Upload the full match",
  "Con compresión, unos 130 MB por minuto de partida": "With compression, about 130 MB per minute of match",
  "Formato de la copia en Drive": "Format of the Drive copy",
  "Se recodifica con la tarjeta gráfica antes de subir: casi igual a la vista y unas 4 veces más liviano. La copia de la PC no cambia.":
    "Re-encoded with the graphics card before uploading: looks almost the same and is about 4 times smaller. The copy on the PC doesn't change.",
  "HEVC (recomendado)": "HEVC (recommended)",
  "AV1 (el más liviano)": "AV1 (smallest)",
  "H.264 (compatible con todo)": "H.264 (works everywhere)",
  "Original, sin comprimir": "Original, uncompressed",
  "Crea un ID de cliente OAuth de tipo": "Create an OAuth client ID of type",
  "App de escritorio": "Desktop app",
  "en Google Cloud Console (API de Google Drive habilitada) y pega aquí sus datos. ggswp solo puede ver los archivos que él mismo sube.":
    "in Google Cloud Console (with the Google Drive API enabled) and paste its details here. ggswp can only see the files it uploads itself.",
  "Drive conectado: {email}": "Drive connected: {email}",
  "Esperando autorización en el navegador…": "Waiting for authorization in the browser…",
  "Conectar Google Drive": "Connect Google Drive",

  // Ajustes: aplicación
  "Aplicación": "Application",
  "Abrir ggswp al iniciar Windows": "Open ggswp when Windows starts",
  "Iniciar minimizado en la bandeja": "Start minimized to the tray",
  "Al cerrar la ventana, seguir en la bandeja": "Keep running in the tray when the window is closed",
  "Necesario para grabar con la ventana cerrada": "Needed to record with the window closed",
  "Puerto para compartir por Wi-Fi": "Port for Wi-Fi sharing",

  // Barra lateral
  "Fortnite cerrado": "Fortnite closed",
  "En el lobby": "In the lobby",
  "Cargando partida": "Loading match",
  "En partida": "In a match",
  "Grabando": "Recording",
  "Esperando partida": "Waiting for a match",
  "{game} abierto": "{game} open",
  "Secciones": "Sections",
  "Partidas": "Matches",
  "Estadísticas": "Stats",
  "Estado de captura": "Capture status",
  "Estado": "Status",
  "Tiempo": "Time",
  "Tipo": "Type",
  "partida": "match",
  "cerrado": "closed",
  "Búfer": "Buffer",
  "apagado": "off",
  "{n} s listo": "{n} s ready",
  "en espera": "standby",
  "Detener": "Stop",
  "Atajos": "Hotkeys",
  "Guardar {n} s": "Save {n} s",
  "Grabar / detener": "Record / stop",
  "Menú en el juego": "In-game menu",
  "Drive en pausa mientras juegas": "Drive paused while you play",
  "Comprimiendo para Drive": "Compressing for Drive",
  "Bajando de Drive": "Downloading from Drive",
  "Subiendo a Drive": "Uploading to Drive",
  "conectado": "connected",
  "Google Drive sin conectar": "Google Drive not connected",
  "Conectar": "Connect",

  // Aviso de actualización
  "Actualización": "Update",
  "Recordármelo después": "Remind me later",
  "No se pudo actualizar: {error}": "Couldn't update: {error}",
  "Descargando · {pct} %": "Downloading · {pct}%",
  "Descargando…": "Downloading…",
  "Espera a que termine la grabación": "Wait for the recording to finish",
  "Disponible al terminar de grabar": "Available when recording ends",
  "Actualizar y reiniciar": "Update and restart",

  // Atajo de teclado y diálogo de renombrar
  "Presiona la combinación…": "Press the key combination…",
  "Sin asignar": "Not set",
  "Cerrar": "Close",
  "Cancelar": "Cancel",
  "Guardando…": "Saving…",
  "Guardar": "Save",

  // Subidas
  "Subida fallida: {error}": "Upload failed: {error}",
};

export default dict;
