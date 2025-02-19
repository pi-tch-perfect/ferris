use serde::{Deserialize, Serialize};
use std::{fs::File, io::BufReader, path::Path, sync::Arc};
use tokio::sync::oneshot;
use tracing::{debug, error, info, trace};

use crate::lib::{
    dash_processor::{DashProcessor, ProcessingMode},
    yt_downloader::{VideoProcessError, YtDownloader},
};

#[derive(Serialize, Deserialize)]
struct VideoStatus {
    segments: u32,
    is_key_changeable: bool,
}

pub enum VideoDlActorMessage {
    DownloadVideo {
        yt_link: String,
        name: String,
        is_key_changeable: bool,
        respond_to: oneshot::Sender<Result<String, VideoProcessError>>,
    },
}

struct VideoDlActor {
    receiver: async_channel::Receiver<VideoDlActorMessage>,
    downloader: Arc<YtDownloader>,
    base_dir: String,
    consumer_id: u8,
}

impl VideoDlActor {
    fn new(
        receiver: async_channel::Receiver<VideoDlActorMessage>,
        base_dir: String,
        video_downloader: Arc<YtDownloader>,
        consumer_id: u8,
    ) -> Self {
        trace!("Initializing VideoDlActor consumer {}", consumer_id);
        VideoDlActor {
            receiver,
            base_dir,
            downloader: video_downloader,
            consumer_id,
        }
    }

    async fn handle_message(&mut self, msg: VideoDlActorMessage) {
        info!(
            "Consumer {} received video download message",
            self.consumer_id
        );

        match msg {
            VideoDlActorMessage::DownloadVideo {
                yt_link,
                name,
                is_key_changeable,
                respond_to,
            } => {
                info!(
                    "Consumer {} starting to process video from {} to path {}",
                    self.consumer_id, yt_link, name
                );

                let video_path = format!("{}/{}", self.base_dir, name);

                info!(
                    "video exists: {}",
                    self.video_exists(&video_path, is_key_changeable)
                );
                if Path::new(&video_path).exists()
                    && self.video_exists(&video_path, is_key_changeable)
                {
                    info!(
                        "Consumer {} found existing processed video {} in path {}/{}",
                        self.consumer_id, yt_link, self.base_dir, name
                    );
                    let _ = respond_to.send(Ok(String::from("success")));
                } else {
                    if Path::new(&video_path).exists() {
                        trace!(
                            "Consumer {} clearing existing folder at {}",
                            self.consumer_id,
                            video_path
                        );
                        if let Err(e) = std::fs::remove_dir_all(&video_path) {
                            error!(
                                "Consumer {} failed to clear folder {}: {}",
                                self.consumer_id, video_path, e
                            );
                            let _ = respond_to.send(Err(VideoProcessError::PitchShiftError(
                                format!("Failed to clear existing folder: {}", e)
                            )));
                            return;
                        }
                    }

                    let result = self
                        .process_video(&yt_link, &self.base_dir, &name, &is_key_changeable, &4)
                        .await;
                    info!(
                        "Consumer {} finished processing video from {}: {:?}",
                        self.consumer_id,
                        yt_link,
                        if result.is_ok() { "success" } else { "failed" }
                    );
                    let _ = respond_to.send(result);
                }
            }
        }
    }

    fn video_exists(&self, base_path: &str, is_key_changeable: bool) -> bool {
        let status_path = format!("{}/status.json", base_path);

        // Check if status.json exists
        if !Path::new(&status_path).exists() {
            trace!(
                "Consumer {} - status.json not found at {}",
                self.consumer_id,
                status_path
            );
            return false;
        }

        // Read and parse status.json
        let file = match File::open(&status_path) {
            Ok(file) => file,
            Err(e) => {
                trace!(
                    "Consumer {} - Failed to open status.json: {}",
                    self.consumer_id,
                    e
                );
                return false;
            }
        };

        let status: VideoStatus = match serde_json::from_reader(BufReader::new(file)) {
            Ok(status) => status,
            Err(e) => {
                trace!(
                    "Consumer {} - Failed to parse status.json: {}",
                    self.consumer_id,
                    e
                );
                return false;
            }
        };

        // Check key_changeable compatibility
        if is_key_changeable && !status.is_key_changeable {
            trace!(
                "Consumer {} - Key change requested but existing file doesn't support it",
                self.consumer_id
            );
            return false;
        }

        // Check if corresponding chunk file exists
        let chunk_path = format!("{}/chunk-stream1-{:05}.m4s", base_path, status.segments);
    
        debug!("chunk_path: {}", chunk_path);

        let chunk_exists = Path::new(&chunk_path).exists();

        trace!(
            "Consumer {} - Checking for chunk file: {} - {}",
            self.consumer_id,
            chunk_path,
            if chunk_exists { "found" } else { "not found" }
        );

        chunk_exists
    }

    async fn process_video(
        &self,
        yt_link: &str,
        base_dir: &str,
        name: &str,
        is_key_changeable: &bool,
        segment_duration: &u32,
    ) -> Result<String, VideoProcessError> {
        trace!(
            "Consumer {} starting download of {}",
            self.consumer_id,
            yt_link
        );
        let video_metadata = self.downloader.download(yt_link, base_dir, name).await?;
        let (dir, file_name, extension, duration_seconds) = (
            video_metadata.directory,
            video_metadata.filename,
            video_metadata.extension,
            video_metadata.duration_seconds,
        );

        let status_file_path = format!("{}/status.json", dir);
        let status = VideoStatus {
            segments: (duration_seconds / (*segment_duration as f64)).ceil() as u32,
            is_key_changeable: *is_key_changeable,
        };

        match File::create(&status_file_path) {
            Ok(file) => {
                if let Err(e) = serde_json::to_writer_pretty(file, &status) {
                    trace!(
                        "Consumer {} failed to write status file {}: {}",
                        self.consumer_id,
                        status_file_path,
                        e
                    );
                    return Err(VideoProcessError::PitchShiftError(format!(
                        "Failed to write status file: {}",
                        e
                    )));
                }
                trace!(
                    "Consumer {} wrote status file with {} segments to {}",
                    self.consumer_id,
                    status.segments,
                    status_file_path
                );
            }
            Err(e) => {
                trace!(
                    "Consumer {} failed to create status file {}: {}",
                    self.consumer_id,
                    status_file_path,
                    e
                );
                return Err(VideoProcessError::PitchShiftError(format!(
                    "Failed to create status file: {}",
                    e
                )));
            }
        }

        trace!(
            "Consumer {} completed download. Dir: {}, File: {}.{}",
            self.consumer_id,
            dir,
            file_name,
            extension
        );

        let dash_processor = DashProcessor::new(4);
        let mode;

        if *is_key_changeable {
            trace!(
                "Consumer {} starting dash processing with pitch shifting for {}",
                self.consumer_id,
                file_name
            );
            mode = ProcessingMode::PitchShift(vec![-3, -2, -1, 0, 1, 2, 3])
        } else {
            trace!(
                "Consumer {} starting dash processing with no pitch shifting for {}",
                self.consumer_id,
                file_name
            );
            mode = ProcessingMode::Copy;
        }

        match dash_processor.execute(
            &format!("{}/{}.{}", dir, file_name, extension),
            &format!("{}/{}.mpd", dir, file_name),
            &mode,
        ) {
            Ok(_) => {
                trace!(
                    "Consumer {} completed pitch shifting for {}",
                    self.consumer_id,
                    file_name
                );
                Ok(format!("{}/{}.{}", dir, file_name, extension))
            }
            Err(e) => {
                trace!(
                    "Consumer {} failed pitch shifting for {}: {}",
                    self.consumer_id,
                    file_name,
                    e
                );
                Err(VideoProcessError::PitchShiftError(format!(
                    "Pitch shift failed: {}",
                    e
                )))
            }
        }
    }
}

async fn run_video_dl_actor(mut actor: VideoDlActor) {
    info!(
        "Starting video download actor consumer {}",
        actor.consumer_id
    );
    loop {
        trace!(
            "Consumer {} waiting for message. Channel capacity: {}, len: {}",
            actor.consumer_id,
            actor.receiver.capacity().unwrap(),
            actor.receiver.len()
        );

        match actor.receiver.recv().await {
            Ok(msg) => {
                trace!("Total receiver count: {}", actor.receiver.receiver_count());

                trace!(
                    "Consumer {} received message. Channel capacity: {}, len: {}",
                    actor.consumer_id,
                    actor.receiver.capacity().unwrap(),
                    actor.receiver.len()
                );
                actor.handle_message(msg).await;
                trace!(
                    "Consumer {} completed processing. Channel capacity: {}, len: {}",
                    actor.consumer_id,
                    actor.receiver.capacity().unwrap(),
                    actor.receiver.len()
                );
            }
            Err(e) => {
                error!(
                    "Consumer {} channel closed, shutting down: {}",
                    actor.consumer_id, e
                );
                break;
            }
        }
    }
    info!("Consumer {} shutting down", actor.consumer_id);
}

#[derive(Clone)]
pub struct VideoDlActorHandle {
    sender: async_channel::Sender<VideoDlActorMessage>,
}

impl VideoDlActorHandle {
    pub fn new(base_dir: String, yt_downloader: Arc<YtDownloader>) -> Self {
        trace!("Initializing VideoDlActorHandle");
        let (sender, receiver) = async_channel::bounded(100);
        trace!(
            "Created channel with capacity: {}",
            sender.capacity().unwrap()
        );

        const NUM_CONSUMERS: u8 = 5;
        trace!("Starting {} consumers", NUM_CONSUMERS);
        for consumer_id in 0..NUM_CONSUMERS {
            trace!("Spawning consumer {}", consumer_id);
            let actor = VideoDlActor::new(
                receiver.clone(),
                base_dir.clone(),
                yt_downloader.clone(),
                consumer_id,
            );
            tokio::spawn(run_video_dl_actor(actor));
        }
        trace!("All consumers spawned");
        trace!("Total receiver count: {}", receiver.receiver_count());

        Self { sender }
    }

    pub async fn download_video(
        &self,
        yt_link: String,
        name: String,
        pitch_shift: bool,
    ) -> Result<String, VideoProcessError> {
        trace!(
            "Requesting video download for {} (channel len: {})",
            yt_link,
            self.sender.len()
        );

        let (send, recv) = oneshot::channel();
        let msg = VideoDlActorMessage::DownloadVideo {
            yt_link: yt_link.clone(),
            name: name.clone(),
            is_key_changeable: pitch_shift.clone(),
            respond_to: send,
        };

        trace!(
            "Sending download request for {} to video download actor (channel len: {})",
            yt_link,
            self.sender.len()
        );
        let _ = self.sender.send(msg).await;

        trace!(
            "Message sent for {}. Channel status - len: {}, capacity: {}",
            yt_link,
            self.sender.len(),
            self.sender.capacity().unwrap()
        );

        trace!("Awaiting response for {}", yt_link);
        let result = recv.await.expect("Actor task has been killed");
        trace!(
            "Received response for {}: {:?}",
            yt_link,
            if result.is_ok() { "success" } else { "failed" }
        );
        result
    }
}
