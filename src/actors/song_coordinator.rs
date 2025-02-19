use std::{collections::VecDeque, fmt::Display, sync::Arc, usize};
use strum::Display;
use thiserror::Error;

use tokio::sync::{self, mpsc, oneshot};
use tracing::{error, warn};
use uuid::Uuid;

use crate::routes::karaoke::SseEvent;

fn serialize_uuid<S>(uuid: &Uuid, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(uuid.to_string().as_str())
}

#[derive(Clone, serde::Serialize, PartialEq, Display)]
pub enum QueuedSongStatus {
    InProgress,
    Failed,
    Success,
}

#[derive(Clone, serde::Serialize)]
pub struct Song {
    pub name: String,
    #[serde(serialize_with = "serialize_uuid")]
    pub uuid: Uuid,
    pub yt_link: String,
    pub status: QueuedSongStatus,
    pub is_key_changeable: bool
}

impl Display for Song {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Song: {{ name: {}, uuid: {}, yt_link: {}, status: {} }}",
            self.name, self.uuid, self.yt_link, self.status
        )
    }
}

impl Song {
    pub fn new(name: String, yt_link: String, status: QueuedSongStatus, is_key_changeable: bool) -> Self {
        Song {
            name: name.to_string(),
            uuid: Uuid::new_v4(),
            yt_link,
            status,
            is_key_changeable
        }
    }
}

impl PartialEq for Song {
    fn eq(&self, other: &Self) -> bool {
        self.uuid == other.uuid || self.name == other.name
    }
}

struct SongActor {
    receiver: mpsc::Receiver<SongActorMessage>,
    song_deque: VecDeque<Song>,
    current_key: i8,
    sse_broadcaster: Arc<sync::broadcast::Sender<SseEvent>>,
}

pub enum SongActorMessage {
    QueueSong {
        song: Song,
        respond_to: oneshot::Sender<Result<(), SongCoordinatorError>>,
    },
    RemoveSong {
        song_uuid: Uuid,
        respond_to: oneshot::Sender<()>,
    },
    PopSong {
        respond_to: oneshot::Sender<Option<Song>>,
    },
    Reposition {
        song_uuid: Uuid,
        position: usize,
        respond_to: oneshot::Sender<Result<(), SongCoordinatorError>>,
    },
    Current {
        respond_to: oneshot::Sender<Result<Option<Song>, SongCoordinatorError>>,
    },
    GetQueue {
        respond_to: oneshot::Sender<Result<VecDeque<Song>, SongCoordinatorError>>,
    },
    KeyUp {
        respond_to: oneshot::Sender<Result<i8, SongCoordinatorError>>,
    },
    KeyDown {
        respond_to: oneshot::Sender<Result<i8, SongCoordinatorError>>,
    },
    GetKey {
        respond_to: oneshot::Sender<Result<i8, SongCoordinatorError>>,
    },
    UpdateSongStatus {
        song_uuid: Uuid,
        status: QueuedSongStatus,
        respond_to: oneshot::Sender<Result<(), SongCoordinatorError>>,
    },
}

#[derive(Error, Debug)]
pub enum SongCoordinatorError {
    #[error("unable to queue song: {uuid}")]
    QueueSongFailed { uuid: Uuid },

    #[error("song already queued: {name}")]
    SongAlreadyQueued { name: String },

    #[error("unable to remove song: {uuid}")]
    RemoveSongFailed { uuid: Uuid },

    #[error("unable to pop song")]
    PopSongFailed,

    #[error("unable to reposition song: {uuid}")]
    RepositionSongFailed { uuid: Uuid },

    #[error("unable to get current song")]
    GetCurrentSongFailed,

    #[error("unable to get queue")]
    GetQueueFailed,

    #[error("unable to key up")]
    KeyUpFailed,

    #[error("unable to key down")]
    KeyDownFailed,

    #[error("unable to update song status for: {uuid}")]
    UpdateSongStatusFailed { uuid: Uuid },

    #[error("failed to broadcast SSE event")]
    SseBroadcastFailed,
}

impl SongActor {
    fn new(
        receiver: mpsc::Receiver<SongActorMessage>,
        sse_broadcaster: Arc<sync::broadcast::Sender<SseEvent>>,
    ) -> Self {
        SongActor {
            receiver,
            sse_broadcaster,
            song_deque: VecDeque::new(),
            current_key: 0,
        }
    }

    async fn handle_message(&mut self, msg: SongActorMessage) {
        match msg {
            SongActorMessage::QueueSong { song, respond_to } => {
                if self.song_deque.contains(&song) {

                    let _ = respond_to.send(Err(SongCoordinatorError::SongAlreadyQueued { name: song.name }));
                } else {
                    self.song_deque.push_back(song.clone());

                    match self.sse_broadcaster.send(SseEvent::QueueUpdated {
                        queue: self.song_deque.clone(),
                    }) {
                        Ok(_) => {
                            let _ = respond_to.send(Ok(()));
                        }
                        Err(err) => {
                            // Remove the song since broadcasting failed
                            warn!("failed to broadcast SSE event for queue update event for song: {} with error: {}", song.uuid, err);
                            let _ = respond_to.send(Ok(()));
                        }
                    }
                }
            }
            SongActorMessage::RemoveSong {
                song_uuid,
                respond_to,
            } => {
                if let Some(index) = self.song_deque.iter().position(|x| x.uuid == song_uuid) {
                    self.song_deque.remove(index);
                }

                match self.sse_broadcaster.send(SseEvent::QueueUpdated {
                    queue: self.song_deque.clone(),
                }) {
                    Ok(_) => {
                        let _ = respond_to.send(());
                    }
                    Err(err) => {
                        warn!(
                            "failed to broadcast SSE event for queue update event for song: {} with error: {}", 
                            song_uuid, 
                            err
                        );
                        let _ = respond_to.send(());
                    }
                }
            }
            SongActorMessage::PopSong { respond_to } => {
                // remove all failed songs while getting the next one
                let next_song = self.song_deque.pop_front();

                self.current_key = 0;

                match self.sse_broadcaster.send(SseEvent::QueueUpdated {
                    queue: self.song_deque.clone(),
                }) {
                    Ok(_) => {
                        let _ = respond_to.send(next_song.clone());
                    }
                    Err(err) => {
                        warn!("failed to broadcast SSE event for queue update event with error: {}", err);
                        let _ = respond_to.send(next_song.clone());
                    }
                }
            }
            SongActorMessage::Reposition {
                song_uuid,
                position,
                respond_to,
            } => {
                if let Some(current_index) = self.song_deque.iter().position(|x| x.uuid == song_uuid) {
                    let song = self.song_deque.remove(current_index).unwrap();
                    let new_position = position.min(self.song_deque.len());
                    self.song_deque.insert(new_position, song);
                    
                    match self.sse_broadcaster.send(SseEvent::QueueUpdated {
                        queue: self.song_deque.clone(),
                    }) {
                        Ok(_) => {
                            let _ = respond_to.send(Ok(()));
                        }
                        Err(err) => {
                            warn!(
                                "failed to broadcast SSE event for queue update event for song: {} with error: {}", 
                                song_uuid, 
                                err
                            );
                            let _ = respond_to.send(Ok(()));
                        }
                    }
                } else {
                    let _ = respond_to.send(Ok(()));
                }
            }
            SongActorMessage::Current { respond_to } => {
                let _ = respond_to.send(Ok(self.song_deque.front().cloned()));
            }
            SongActorMessage::GetQueue { respond_to } => {
                let _ = respond_to.send(Ok(self.song_deque.clone()));
            }
            SongActorMessage::KeyUp { respond_to } => {
                if self.current_key >= 3 {
                    // TODO fix this and grab it from some settings descriptor
                    let _ = respond_to.send(Err(SongCoordinatorError::KeyUpFailed));
                } else {
                    self.current_key += 1;
                    let _ = self.sse_broadcaster.send(SseEvent::KeyChange {
                        current_key: self.current_key,
                    });

                    let _ = respond_to.send(Ok(self.current_key));
                }
            }
            SongActorMessage::KeyDown { respond_to } => {
                if self.current_key <= -3 {
                    // TODO fix this and grab it from some settings descriptor
                    let _ = respond_to.send(Err(SongCoordinatorError::KeyDownFailed));
                } else {
                    self.current_key -= 1;
                    let _ = self.sse_broadcaster.send(SseEvent::KeyChange {
                        current_key: self.current_key,
                    });

                    let _ = respond_to.send(Ok(self.current_key));
                }
            }
            SongActorMessage::GetKey { respond_to } => {
                let _ = respond_to.send(Ok(self.current_key));
            }
            SongActorMessage::UpdateSongStatus {
                song_uuid,
                status,
                respond_to,
            } => {
                if let Some(song) = self
                    .song_deque
                    .iter_mut()
                    .find(|song| song.uuid == song_uuid)
                {
                    song.status = status;

                    let _ = self.sse_broadcaster.send(SseEvent::QueueUpdated {
                        queue: self.song_deque.clone(),
                    });

                    let _ = respond_to.send(Ok(()));
                } else {
                    let _ = respond_to.send(Err(SongCoordinatorError::UpdateSongStatusFailed {
                        uuid: (song_uuid),
                    }));
                }
            }
        }
    }
}

async fn run_song_actor(mut actor: SongActor) {
    while let Some(msg) = actor.receiver.recv().await {
        actor.handle_message(msg).await;
    }
}

#[derive(Clone)]
pub struct SongActorHandle {
    sender: mpsc::Sender<SongActorMessage>,
}

impl SongActorHandle {
    pub fn new(sse_broadcaster: Arc<sync::broadcast::Sender<SseEvent>>) -> Self {
        let (sender, receiver) = mpsc::channel(8);
        let song_actor = SongActor::new(receiver, sse_broadcaster);
        tokio::spawn(run_song_actor(song_actor));

        Self { sender }
    }

    pub async fn queue_song(&self, song: Song) -> Result<(), SongCoordinatorError> {
        let (send, recv) = oneshot::channel();
        let msg = SongActorMessage::QueueSong {
            song,
            respond_to: send,
        };

        let _ = self.sender.send(msg).await;
        recv.await.expect("Actor task has been killed")
    }

    pub async fn update_song_status(
        &self,
        song_uuid: Uuid,
        new_status: QueuedSongStatus,
    ) -> Result<(), SongCoordinatorError> {
        let (send, recv) = oneshot::channel();
        let msg = SongActorMessage::UpdateSongStatus {
            song_uuid,
            status: new_status,
            respond_to: send,
        };

        let _ = self.sender.send(msg).await;
        recv.await.expect("Actor task has been killed")
    }

    pub async fn remove_song(&self, song_uuid: Uuid) {
        let (send, recv) = oneshot::channel();
        let msg = SongActorMessage::RemoveSong {
            song_uuid,
            respond_to: send,
        };

        let _ = self.sender.send(msg).await;
        recv.await.expect("Actor task has been killed")
    }

    pub async fn pop_song(&self) -> Option<Song> {
        let (send, recv) = oneshot::channel();
        let msg = SongActorMessage::PopSong { respond_to: send };

        let _ = self.sender.send(msg).await;
        recv.await.expect("Actor task has been killed")
    }

    pub async fn reposition_song(
        &self,
        song_uuid: Uuid,
        position: usize,
    ) -> Result<(), SongCoordinatorError> {
        let (send, recv) = oneshot::channel();
        let msg = SongActorMessage::Reposition {
            song_uuid,
            position,
            respond_to: send,
        };

        let _ = self.sender.send(msg).await;
        recv.await.expect("Actor task has been killed")
    }

    pub async fn current_song(&self) -> Result<Option<Song>, SongCoordinatorError> {
        let (send, recv) = oneshot::channel();
        let msg = SongActorMessage::Current { respond_to: send };

        let _ = self.sender.send(msg).await;
        recv.await.expect("Actor task has been killed")
    }

    pub async fn get_queue(&self) -> Result<VecDeque<Song>, SongCoordinatorError> {
        let (send, recv) = oneshot::channel();
        let msg = SongActorMessage::GetQueue { respond_to: send };

        let _ = self.sender.send(msg).await;
        recv.await.expect("Actor task has been killed")
    }

    pub async fn key_up(&self) -> Result<i8, SongCoordinatorError> {
        let (send, recv) = oneshot::channel();
        let msg = SongActorMessage::KeyUp { respond_to: send };

        let _ = self.sender.send(msg).await;
        recv.await.expect("Actor task has been killed")
    }

    pub async fn key_down(&self) -> Result<i8, SongCoordinatorError> {
        let (send, recv) = oneshot::channel();
        let msg = SongActorMessage::KeyDown { respond_to: send };

        let _ = self.sender.send(msg).await;
        recv.await.expect("Actor task has been killed")
    }

    pub async fn get_key(&self) -> Result<i8, SongCoordinatorError> {
        let (send, recv) = oneshot::channel();
        let msg = SongActorMessage::GetKey { respond_to: send };

        let _ = self.sender.send(msg).await;
        recv.await.expect("Actor task has been killed")
    }
}
