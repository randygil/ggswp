//! Prueba de punta a punta: `cargo test --test pipeline -- --ignored --nocapture`
use ggswp_lib::capture::{self, Session, SessionConfig};
use ggswp_lib::config::Settings;
use ggswp_lib::engine::{analyze, video_options};
use ggswp_lib::fortnite::{log::LogTailer, replay};
use ggswp_lib::library::{Library, MatchContext, MatchMeta, MatchStatus, probe};

#[test]
#[ignore]
fn record_and_analyze() {
    let settings = Settings::default();
    let lib = Library::new(settings.library_dir.clone());
    let id = "0000-prueba-pipeline".to_string();
    let _ = std::fs::remove_dir_all(lib.dir(&id));
    let ffmpeg = std::path::PathBuf::from("ffmpeg");

    let cfg = SessionConfig { ffmpeg: ffmpeg.clone(), video: video_options(&settings, 0), system_audio: true, microphone: false, mic_volume: 1.0, game_pid: None };
    let s = Session::start(lib.dir(&id), cfg).unwrap();
    std::thread::sleep(std::time::Duration::from_secs(15));
    let raw = s.stop().unwrap();
    let video = lib.file(&id, "match.mp4");
    capture::finalize(&ffmpeg, &raw, &video).unwrap();
    capture::remove_raw(&raw);
    let p = probe(&ffmpeg, &video).unwrap();

    // Replay real más reciente, desplazado para que la primera eliminación propia caiga en el video.
    let dir = replay::demos_dir().unwrap();
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    files.sort_by_key(|f| std::fs::metadata(f).unwrap().modified().unwrap());
    let rep = replay::parse_file(files.last().unwrap()).unwrap();
    let me = LogTailer::new(ggswp_lib::fortnite::log::log_path().unwrap())
        .catch_up()
        .into_iter()
        .find_map(|t| match t.event { ggswp_lib::fortnite::log::LogEvent::MatchLoading { account_id, .. } => account_id, _ => None })
        .unwrap();
    let first = rep.eliminations.iter().find(|e| e.eliminator.is(&me)).unwrap();
    let video_start = rep.started_at + chrono::Duration::milliseconds(first.time_ms as i64 - 8_000);

    let ctx = MatchContext { account_id: Some(me), presence: Some("Prueba de pipeline - 1 restante(s)".into()), ..Default::default() };
    let mut meta = MatchMeta {
        id: id.clone(),
        created_at: chrono::Utc::now(),
        duration_s: p.duration,
        width: p.width,
        height: p.height,
        size_bytes: std::fs::metadata(&video).unwrap().len(),
        video: Some("match.mp4".into()),
        mode: ctx.mode_name(),
        ..Default::default()
    };
    analyze(&ffmpeg, &lib, &mut meta, &ctx, Some(&rep), video_start, &settings, Vec::new(), &|_, _| {}).unwrap();
    meta.title = format!("Prueba de pipeline · {}", meta.title);
    meta.status = MatchStatus::Ready;
    lib.save(&meta).unwrap();
    println!("{} {}x{} {:.1}s events={:?}", meta.title, meta.width, meta.height, meta.duration_s, meta.events);
    println!("clips={:?}", meta.clips.iter().map(|c| (&c.title, c.start, c.end)).collect::<Vec<_>>());
    assert!(meta.kills >= 1 && !meta.clips.is_empty() && meta.thumb.is_some());
}
