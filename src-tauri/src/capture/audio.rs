//! Captura de audio WASAPI: loopback del dispositivo de salida (lo que escuchas), micrófono
//! o loopback de un solo proceso (el juego, o todo menos el juego).
//!
//! Cada paquete trae su timestamp QPC; se coloca en su posición exacta respecto de `t0`
//! y los huecos se rellenan con silencio (el loopback no entrega paquetes cuando no suena
//! nada). Así el WAV queda alineado al mismo reloj que el video.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use anyhow::{Context, Result, anyhow};
use wasapi::{AudioClient, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat, initialize_mta};

use super::win::qpc_100ns;

const RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
/// Tolerancia antes de insertar silencio o descartar solapamientos (10 ms).
const SLACK_FRAMES: i64 = (RATE / 100) as i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    SystemLoopback,
    Microphone,
    /// Solo lo que suena desde ese proceso y sus hijos (Windows 10 2004 o superior).
    Process(u32),
    /// Todo lo que suena menos ese proceso y sus hijos.
    AllBut(u32),
}

pub struct AudioCapture {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<u64>>>,
    pub path: PathBuf,
}

impl AudioCapture {
    /// Empieza a capturar. El frame 0 del WAV corresponde al instante QPC `t0`.
    pub fn start(source: Source, path: PathBuf, t0: u64) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();
        let thread = {
            let stop = stop.clone();
            let path = path.clone();
            std::thread::Builder::new().name(format!("audio-{source:?}")).spawn(move || {
                let r = run(source, &path, t0, &stop, &ready_tx);
                if let Err(e) = &r {
                    let _ = ready_tx.send(Err(anyhow!("{e:#}")));
                }
                r
            })?
        };
        match ready_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Self { stop, thread: Some(thread), path }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(anyhow!("el dispositivo de audio no respondió")),
        }
    }

    /// Detiene y devuelve los frames escritos.
    pub fn stop(mut self) -> Result<u64> {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().map_err(|_| anyhow!("hilo de audio caído"))?
    }
}

fn run(source: Source, path: &PathBuf, t0: u64, stop: &AtomicBool, ready: &std::sync::mpsc::Sender<Result<()>>) -> Result<u64> {
    initialize_mta().ok().context("COM")?;
    let format = WaveFormat::new(16, 16, SampleType::Int, RATE as usize, CHANNELS as usize, None);
    let device = |d: Direction| -> Result<(AudioClient, i64)> {
        let client = DeviceEnumerator::new()?.get_default_device(d)?.get_iaudioclient()?;
        let (default_period, _) = client.get_device_period()?;
        Ok((client, default_period * 4))
    };
    // El loopback de proceso no tiene dispositivo: el tamaño del búfer lo decide Windows.
    let (mut client, buffer_duration_hns) = match source {
        Source::SystemLoopback => device(Direction::Render)?,
        Source::Microphone => device(Direction::Capture)?,
        Source::Process(pid) => (AudioClient::new_application_loopback_client(pid, true)?, 0),
        Source::AllBut(pid) => (AudioClient::new_application_loopback_client(pid, false)?, 0),
    };
    client.initialize_client(&format, Direction::Capture, StreamMode::EventsShared { autoconvert: true, buffer_duration_hns }, None)?;
    let event = client.set_get_eventhandle()?;
    let capture = client.get_audiocaptureclient()?;

    let spec = hound::WavSpec { channels: CHANNELS, sample_rate: RATE, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut wav = hound::WavWriter::create(path, spec)?;
    let block = format.get_blockalign() as usize;
    let mut written: i64 = 0;
    let mut queue: VecDeque<u8> = VecDeque::new();

    let pos_of = |qpc: u64| -> i64 { ((qpc as i128 - t0 as i128) * RATE as i128 / 10_000_000) as i64 };

    client.start_stream()?;
    let _ = ready.send(Ok(()));

    let write_silence = |wav: &mut hound::WavWriter<_>, frames: i64| -> Result<()> {
        let mut w = wav.get_i16_writer((frames.max(0) as u32) * CHANNELS as u32);
        for _ in 0..frames.max(0) * CHANNELS as i64 {
            w.write_sample(0i16);
        }
        w.flush()?;
        Ok(())
    };

    while !stop.load(Ordering::SeqCst) {
        let _ = event.wait_for_event(100);
        loop {
            queue.clear();
            let info = match capture.read_from_device_to_deque(&mut queue) {
                Ok(i) => i,
                Err(e) => return Err(anyhow!("lectura de audio: {e}")),
            };
            if queue.is_empty() {
                break;
            }
            let frames = (queue.len() / block) as i64;
            let at = if info.timestamp > 0 { pos_of(info.timestamp) } else { written };
            let mut skip: i64 = 0;
            if at > written + SLACK_FRAMES {
                write_silence(&mut wav, at - written)?;
                written = at;
            } else if at + SLACK_FRAMES < written {
                // Paquete que se solapa con lo ya escrito: descartamos la parte repetida.
                skip = (written - at).min(frames);
            }
            if at + frames < 0 {
                continue; // anterior a t0
            }
            if at < 0 {
                skip = skip.max(-at);
            }
            let bytes = queue.make_contiguous();
            let start = skip as usize * block;
            let count = (frames - skip) as u32 * CHANNELS as u32;
            let mut w = wav.get_i16_writer(count);
            for s in bytes[start..].chunks_exact(2) {
                w.write_sample(i16::from_le_bytes([s[0], s[1]]));
            }
            w.flush()?;
            written += frames - skip;
        }
        // Silencio prolongado: avanzamos el archivo con el reloj para no acumular desfase.
        let now = pos_of(qpc_100ns());
        if now - written > RATE as i64 / 2 {
            let target = now - RATE as i64 / 4;
            write_silence(&mut wav, target - written)?;
            written = target;
        }
    }
    client.stop_stream()?;
    let now = pos_of(qpc_100ns());
    if now > written {
        write_silence(&mut wav, now - written)?;
        written = now;
    }
    wav.finalize()?;
    Ok(written as u64)
}
