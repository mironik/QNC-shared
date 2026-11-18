//! Shot analysis of the clips of a project (user 2026-10-27: Video AI in AI Media Assist; a
//! background job of its own after the filmstrip, lower than the player, on by the project's
//! setting `ai.enabled` "AI shot analysis and SubClips"). The background worker runs this block
//! after the filmstrips and waves; it knows no form.
//!
//! For each imported clip not yet analysed: one intra picture of its original about every
//! second, read through the original's own MXF index (the card is only read, nothing else of
//! the file), decoded small and grey through the decoder catalog, the shots found by
//! `qnc-shot-detect`, kept in the project database by `qnc-shot-db` under the caller's lease.
//! The clip a preview has open goes first; while the player plays it waits. An original that
//! is not on this computer is left for a later run; one without a usable index (not MXF, a
//! format not read yet) is recorded with that reason.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use qnc_content_store::{Access, ContentTarget};
use qnc_db_broker::{ProjectDbTarget, WriteLease};
use qnc_shot_db::{ClipShots, PartRecord, ShotRecord};
use qnc_work_settings::{SettingsReader, WorkSettings};

mod keys;
pub use keys::KeyPictures;

pub const MODULE_ID: &str = "qnc.module.shot-analysis";

/// Where a media URI is on this computer; `None` when it is not here.
pub type LocalMedia<'a> = dyn Fn(&str) -> Option<PathBuf> + 'a;

/// The pictures a shot analysis looks at: their size and at most how many a second.
const PICTURE: [u32; 2] = qnc_filmstrip::handed::THUMB_SIZE;
const MOST_A_SECOND: f64 = 4.0;
/// Pictures decoded in one decoder call.
const BATCH: usize = 16;
/// Decoder calls at once at most (user 2026-11-14: the AI shots on several cores; measured on the
/// laptop: 4 at once 2.1x faster than one); never more than half the threads of this computer.
const MOST_DECODERS: usize = 4;

/// How many decoder calls run at once on this computer.
fn decoders() -> usize {
    let threads = std::thread::available_parallelism().map_or(2, usize::from);
    (threads / 2).clamp(1, MOST_DECODERS)
}

/// The project whose clips are analysed: its settings as read from its database.
#[derive(Debug, Clone)]
pub struct ShotProject {
    reader: SettingsReader,
    settings: WorkSettings,
}

impl ShotProject {
    pub fn new(reader: SettingsReader, settings: WorkSettings) -> Self {
        Self { reader, settings }
    }

    /// The active project of the QNC root, read from its database now.
    pub fn active(root: &Path) -> Result<Self, String> {
        let active = qnc_active_project_read::ActiveProjectReader::from_root(root).map_err(|e| e.to_string())?;
        let snapshot = active.read().map_err(|e| e.to_string())?;
        Ok(Self::new(active.settings_reader().clone(), snapshot.settings))
    }
}

/// What the caller says while the analysis runs.
pub struct Pace<'a> {
    /// The player plays now: the analysis waits.
    pub player_plays: &'a dyn Fn() -> bool,
    /// The clip a preview has open, analysed first.
    pub wanted: &'a dyn Fn() -> Option<String>,
    /// Which clips this turn may look at (user 2026-11-12: a clip whose first channel holds a
    /// statement is not analysed); `&|_| true` for every clip.
    pub takes: &'a dyn Fn(&str) -> bool,
    /// The run stops (the project closed, its lease lost).
    pub cancel: &'a AtomicBool,
}

/// Analyses every imported clip of the project not analysed yet; returns how many it kept.
/// Nothing is done when the project has shot analysis off.
pub fn analyse_missing(project: &ShotProject, local_media: &LocalMedia<'_>, lease: Option<WriteLease>, pace: &Pace<'_>) -> Result<usize, String> {
    let (reader, settings) = (&project.reader, &project.settings);
    if !settings.ai_feature(qnc_ai_features::SHOT_ANALYSIS).unwrap_or(false) {
        return Ok(0);
    }
    let shots_db = ProjectDbTarget::for_project(reader, settings)?;
    let analysed = qnc_shot_db::read_analysed(&shots_db)?;
    let catalog = qnc_content_read::ContentReader::for_project(reader, settings)?;
    let waiting = catalog.records_waiting()?;
    let mut remaining: Vec<String> = catalog
        .summaries()?
        .into_iter()
        .filter(|clip| clip.imported && !analysed.contains(&clip.clip_id) && !waiting.contains(&clip.clip_id) && (pace.takes)(&clip.clip_id))
        .map(|clip| clip.clip_id)
        .collect();
    if remaining.is_empty() {
        return Ok(0);
    }
    let content = ContentTarget::for_project(reader, settings)?;
    let mut client = content.open(Access::ReadOnly)?;
    let mut kept = 0;
    while !remaining.is_empty() {
        if !wait_for_player(pace) {
            break;
        }
        let at = (pace.wanted)().and_then(|clip| remaining.iter().position(|id| *id == clip)).unwrap_or(0);
        let clip_id = remaining.remove(at);
        let Some(clip) = client.read(&clip_id)? else { continue };
        let started = Instant::now();
        let original = clip.clip.snapshot.metadata.original.media_uri.clone();
        let Some(path) = local_media(&original) else {
            log(format!("shot-analysis clip={clip_id} skipped: the original is not on this computer"));
            continue;
        };
        let found = analyse(&clip_id, &clip.clip.snapshot, &path, pace);
        if pace.cancel.load(Ordering::Relaxed) {
            break;
        }
        let record = match found {
            Ok(found) => ClipShots { clip_id: clip_id.clone(), media_uri: original, pictures: found.pictures, shots: found.shots, parts: found.parts, error: String::new(), took_ms: 0 },
            Err(error) => ClipShots { clip_id: clip_id.clone(), media_uri: original, pictures: 0, shots: Vec::new(), parts: Vec::new(), error, took_ms: 0 },
        };
        let record = ClipShots { took_ms: started.elapsed().as_millis() as u64, ..record };
        log(format!(
            "shot-analysis clip={clip_id} pictures={} shots={} parts={} ms={} error={}",
            record.pictures,
            record.shots.len(),
            record.parts.len(),
            record.took_ms,
            record.error
        ));
        qnc_shot_db::save(&shots_db, record, lease.clone())?;
        kept += 1;
    }
    Ok(kept)
}

/// Waits while the player plays; false when the run stops.
fn wait_for_player(pace: &Pace<'_>) -> bool {
    while (pace.player_plays)() {
        if pace.cancel.load(Ordering::Relaxed) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    !pace.cancel.load(Ordering::Relaxed)
}

/// The shots of one clip and how many pictures they came from; an error when its original
/// cannot be read this way.
fn analyse(clip_id: &str, snapshot: &qnc_media_records::Snapshot, path: &Path, pace: &Pace<'_>) -> Result<Found, String> {
    let codec = qnc_filmstrip::original_picture_codec(snapshot).ok_or_else(|| qnc_ui_text::text("shots.codec_not_read").to_string())?;
    let targets = qnc_filmstrip::original_picture_frames(snapshot).unwrap_or_default();
    let found = analyse_file_with(clip_id, codec, path, pace, &targets)?;
    // The filmstrip's thumbnails from the pictures decoded here, for the filmstrip worker of this
    // process (user 2026-11-18: AI shots and the filmstrip in one pass).
    if let Some(jpegs) = found.filmstrip.clone() {
        qnc_filmstrip::handed::hand(clip_id, jpegs);
    }
    Ok(found)
}

/// What the analysis of one clip found.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// How many pictures it looked at.
    pub pictures: u32,
    pub shots: Vec<ShotRecord>,
    pub parts: Vec<PartRecord>,
    /// The filmstrip's thumbnails (JPEG, `qnc_filmstrip::handed::THUMB_SIZE`), one for each place
    /// asked, from the decoded picture nearest it; `None` when none was asked or one is missing.
    pub filmstrip: Option<Vec<Vec<u8>>>,
}

/// The shots and usable parts of one MXF or MP4 original at `path` whose pictures are `codec`
/// (`h264`, `mpeg2video`; an MP4's own index names its codec); the file is only read.
pub fn analyse_file(clip_id: &str, codec: &str, path: &Path, pace: &Pace<'_>) -> Result<Found, String> {
    analyse_file_with(clip_id, codec, path, pace, &[])
}

/// As `analyse_file`, also making the filmstrip's thumbnails for the frames `filmstrip` (its places)
/// from the pictures it decodes: no picture is decoded twice.
pub fn analyse_file_with(clip_id: &str, codec: &str, path: &Path, pace: &Pace<'_>, filmstrip: &[u64]) -> Result<Found, String> {
    let mut keys = KeyPictures::open(path)?.ok_or_else(|| qnc_ui_text::text("shots.no_index").to_string())?;
    let codec = keys.codec().unwrap_or(codec);
    let (frames, rate) = (keys.frames, keys.rate);
    // Every intra picture (about two a second for a camera's long GOP), at most
    // `MOST_A_SECOND` a second (an all-intra original); its frame is where it is shown.
    let least_apart = (rate / MOST_A_SECOND).round().max(1.0) as u64;
    let mut entries: Vec<(usize, u64)> = Vec::new();
    for &(id, shown) in &keys.keys {
        if entries.last().is_none_or(|&(_, last)| shown >= last + least_apart) {
            entries.push((id, shown));
        }
    }
    let extractor = qnc_decoder_catalog::installed_filmstrip_extractor().map_err(|e| e.to_string())?.ok_or_else(|| qnc_ui_text::text("shots.no_decoder").to_string())?;
    // The picture each filmstrip place takes: the one looked at nearest it (an intra picture, as
    // the filmstrip itself takes); its decoded JPEG is kept as it is, the filmstrip's size.
    let nearest: Vec<usize> = if entries.is_empty() { Vec::new() } else { filmstrip.iter().map(|&target| nearest_entry(&entries, target)).collect() };
    let mut kept: std::collections::HashMap<usize, Vec<u8>> = Default::default();
    let temp = scratch_dir(clip_id);
    let result: Result<Vec<qnc_shot_detect::Picture>, String> = (|| {
        let mut pictures = Vec::with_capacity(entries.len());
        let batches: Vec<&[(usize, u64)]> = entries.chunks(BATCH).collect();
        // Several batches at once, each decoded by its own decoder call in a folder of its own;
        // the pictures are read one after the other (the file, one reader), the decoding is what
        // takes the time.
        for (g, group) in batches.chunks(decoders()).enumerate() {
            if !wait_for_player(pace) {
                return Err(qnc_ui_text::text("shots.stopped").into());
            }
            let _ = fs::remove_dir_all(&temp);
            let mut work = Vec::with_capacity(group.len());
            for (b, batch) in group.iter().enumerate() {
                let folder = temp.join(format!("b{b}"));
                fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
                let mut inputs = Vec::with_capacity(batch.len());
                for (n, &(id, _)) in batch.iter().enumerate() {
                    let picture = keys.picture(id)?;
                    let input = folder.join(format!("{n:03}.{codec}"));
                    fs::write(&input, picture).map_err(|e| e.to_string())?;
                    inputs.push(input);
                }
                work.push((folder, inputs));
            }
            let decoded: Vec<Result<(), String>> = std::thread::scope(|scope| {
                let running: Vec<_> = work
                    .iter()
                    .map(|(folder, inputs)| scope.spawn(|| extractor.decode_pictures_with_cancel(codec, inputs, PICTURE, folder, pace.cancel).map_err(|e| e.to_string())))
                    .collect();
                running.into_iter().map(|thread| thread.join().unwrap_or_else(|_| Err("a decoder thread failed".into()))).collect()
            });
            for (b, (((folder, _), batch), decoded)) in work.iter().zip(group).zip(decoded).enumerate() {
                decoded?;
                for (n, &(_, shown)) in batch.iter().enumerate() {
                    let file = folder.join(format!("{n:03}.jpg"));
                    let Ok(image) = image::open(&file) else { continue };
                    let entry = (g * decoders() + b) * BATCH + n;
                    if nearest.contains(&entry) {
                        if let Ok(jpeg) = fs::read(&file) {
                            kept.insert(entry, jpeg);
                        }
                    }
                    let grey = image.into_luma8();
                    pictures.push(qnc_shot_detect::Picture { frame: shown, width: grey.width() as usize, height: grey.height() as usize, luma: grey.into_raw() });
                }
            }
        }
        Ok(pictures)
    })();
    let _ = fs::remove_dir_all(&temp);
    let pictures = result?;
    if pictures.is_empty() {
        return Err(qnc_ui_text::text("shots.no_pictures").into());
    }
    let count = pictures.len() as u32;
    let found = qnc_shot_detect::shots(pictures.clone(), frames);
    let parts = qnc_shot_detect::parts(pictures, &found, rate)
        .into_iter()
        .map(|part| PartRecord { from: part.from, to: part.to, camera: part.camera.name().into(), key_frame: part.key_frame })
        .collect();
    let shots = found
        .into_iter()
        .map(|shot| ShotRecord {
            from: shot.from,
            to: shot.to,
            looks: shot.looks.iter().map(|look| look.name().to_string()).collect(),
            key_frame: shot.key_frame,
            brightness: shot.brightness,
            sharpness: shot.sharpness,
            motion: shot.motion,
        })
        .collect();
    let filmstrip = (!nearest.is_empty()).then(|| nearest.iter().map(|entry| kept.get(entry).cloned()).collect::<Option<Vec<_>>>()).flatten();
    Ok(Found { pictures: count, shots, parts, filmstrip })
}

/// The entry whose picture is shown nearest `target`.
fn nearest_entry(entries: &[(usize, u64)], target: u64) -> usize {
    entries.iter().enumerate().min_by_key(|(_, &(_, shown))| shown.abs_diff(target)).map_or(0, |(at, _)| at)
}

fn scratch_dir(clip_id: &str) -> PathBuf {
    let name: String = clip_id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    std::env::temp_dir().join("qnc-shot-analysis").join(format!("{name}-{}", std::process::id()))
}

fn log(line: String) {
    qnc_dev_diagnostics::log_line(qnc_dev_diagnostics::DiagnosticsStream::Player, line);
}
