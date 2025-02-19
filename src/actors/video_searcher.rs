use std::sync::Arc;

use tokio::sync::oneshot;
use tracing::{error, info, trace};

use crate::lib::yt_searcher::{SearchError, SearchResult, YtSearcher};

pub enum VideoSearcherActorMessage {
    SearchVideo {
        query: String,
        respond_to: oneshot::Sender<Result<Vec<SearchResult>, SearchError>>,
    },
}

struct VideoSearcherActor {
    receiver: async_channel::Receiver<VideoSearcherActorMessage>,
    yt_searcher: Arc<YtSearcher>,
    consumer_id: u8,
}

impl VideoSearcherActor {
    fn new(
        receiver: async_channel::Receiver<VideoSearcherActorMessage>,
        yt_searcher: Arc<YtSearcher>,
        consumer_id: u8,
    ) -> Self {
        trace!("Initializing VideoDlActor consumer {}", consumer_id);
        VideoSearcherActor {
            receiver,
            yt_searcher,
            consumer_id,
        }
    }

    async fn handle_message(&mut self, msg: VideoSearcherActorMessage) {
        info!("Consumer {} received video download message", self.consumer_id);

        match msg {
            VideoSearcherActorMessage::SearchVideo {
                query,
                respond_to,
            } => {
                info!("Consumer {} starting to process search query {}", 
                    self.consumer_id, query);

                let result = self.yt_searcher.search(&query).await;

                info!("Consumer {} finished searching for {} result {}", 
                    self.consumer_id, query, 
                    if result.is_ok() { "success" } else { "failed" });
                let _ = respond_to.send(result);
            }
        }
    }
}

async fn run_video_searcher_actor(mut actor: VideoSearcherActor) {
    info!("Starting video searcher actor consumer {}", actor.consumer_id);
    loop {
        trace!("Consumer {} waiting for message. Channel capacity: {}, len: {}", 
            actor.consumer_id, 
            actor.receiver.capacity().unwrap(),
            actor.receiver.len());
            
        match actor.receiver.recv().await {
            Ok(msg) => {

                trace!("Total receiver count: {}", actor.receiver.receiver_count());

                trace!("Consumer {} received message. Channel capacity: {}, len: {}", 
                    actor.consumer_id, 
                    actor.receiver.capacity().unwrap(),
                    actor.receiver.len());
                actor.handle_message(msg).await;
                trace!("Consumer {} completed processing. Channel capacity: {}, len: {}", 
                    actor.consumer_id,
                    actor.receiver.capacity().unwrap(),
                    actor.receiver.len());
                    
            },
            Err(e) => {
                error!("Consumer {} channel closed, shutting down: {}", actor.consumer_id, e);
                break;
            }
        }
    }
    info!("Consumer {} shutting down", actor.consumer_id);
}

#[derive(Clone)]
pub struct VideoSearcherActorHandle {
    sender: async_channel::Sender<VideoSearcherActorMessage>,
}

impl VideoSearcherActorHandle {
    pub fn new(yt_searcher: Arc<YtSearcher>) -> Self {
        trace!("Initializing VideoSearcherActorHandle");
        let (sender, receiver) = async_channel::bounded(100);
        trace!("Created channel with capacity: {}", sender.capacity().unwrap());

        const NUM_CONSUMERS: u8 = 10;
        trace!("Starting {} consumers", NUM_CONSUMERS);
        for consumer_id in 0..NUM_CONSUMERS {
            trace!("Spawning consumer {}", consumer_id);
            let actor = VideoSearcherActor::new(receiver.clone(), yt_searcher.clone(), consumer_id);
            tokio::spawn(run_video_searcher_actor(actor));
        }
        trace!("All consumers spawned");
        trace!("Total receiver count: {}", receiver.receiver_count());

        Self { sender }
    }

    pub async fn search_videos(&self, query: &str) -> Result<Vec<SearchResult>, SearchError> {
        trace!("Requesting searches for {} (channel len: {})", 
            query, 
            self.sender.len());
            
        let (send, recv) = oneshot::channel();
        let msg = VideoSearcherActorMessage::SearchVideo {
            query: query.to_owned(),
            respond_to: send,
        };

        trace!("Sending search request {} to video searcher actor (channel len: {})", 
            query,
            self.sender.len());
        let _ = self.sender.send(msg).await;
        
        trace!("Message sent for {}. Channel status - len: {}, capacity: {}", 
            query,
            self.sender.len(),
            self.sender.capacity().unwrap());
            
        trace!("Awaiting response for {}", query);
        let result = recv.await.expect("Actor task has been killed");
        trace!("Received response for {}: {:?}", 
            query, 
            if result.is_ok() { "success" } else { "failed" });
        result
    }
}