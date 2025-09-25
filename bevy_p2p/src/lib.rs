use async_channel::TryRecvError;
use bevy::prelude::*;
use distributed_topic_tracker::{
    AutoDiscoveryGossip, GossipReceiver, GossipSender, RecordPublisher, TopicId,
};
use iroh::{Endpoint, protocol::Router};
pub use iroh_gossip::api::Event as PeerEvent;
use iroh_gossip::net::Gossip;
use tokio::task::JoinSet;

pub struct Peer2PeerPlugin {
    pub topic: String,
    pub initial_secret: Vec<u8>,
}

impl Plugin for Peer2PeerPlugin {
    fn build(&self, app: &mut App) {
        let (send_tx, send_rx) = async_channel::unbounded();
        let (recv_tx, recv_rx) = async_channel::unbounded();
        let topic_id = TopicId::new(self.topic.clone());
        let initial_secret = self.initial_secret.clone();
        std::thread::spawn(move || {
            peer_main(topic_id, initial_secret, send_rx, recv_tx);
        });

        app.insert_resource(PeerSender(send_tx))
            .insert_resource(PeerReceiver(recv_rx));
    }
}

#[derive(Resource, Clone)]
pub struct PeerSender(async_channel::Sender<Vec<u8>>);

impl PeerSender {
    pub fn send(&self, data: Vec<u8>) -> Result<()> {
        self.0.send_blocking(data)?;
        Ok(())
    }
}

#[derive(Resource, Clone)]
pub struct PeerReceiver(async_channel::Receiver<Result<PeerEvent>>);

impl PeerReceiver {
    pub fn try_recv(&self) -> Option<Result<PeerEvent>> {
        match self.0.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(e) => Some(Err(e.into())),
        }
    }
}

async fn peer_setup(
    topic_id: TopicId,
    initial_secret: Vec<u8>,
) -> Result<(Router, GossipSender, GossipReceiver)> {
    let endpoint = Endpoint::builder().discovery_n0().bind().await?;
    let gossip = Gossip::builder().spawn(endpoint.clone());
    let router = iroh::protocol::Router::builder(endpoint.clone())
        .accept(iroh_gossip::ALPN, gossip.clone())
        .spawn();
    let record_publisher = RecordPublisher::new(
        topic_id,
        endpoint.node_id().public(),
        endpoint.secret_key().secret().clone(),
        None,
        initial_secret,
    );
    let (gossip_sender, gossip_receiver) = gossip
        .subscribe_and_join_with_auto_discovery_no_wait(record_publisher)
        .await?
        .split()
        .await?;
    Ok((router, gossip_sender, gossip_receiver))
}

#[tokio::main]
async fn peer_main(
    topic_id: TopicId,
    initial_secret: Vec<u8>,
    rx: async_channel::Receiver<Vec<u8>>,
    tx: async_channel::Sender<Result<PeerEvent>>,
) {
    match peer_setup(topic_id, initial_secret).await {
        Ok((_router, gossip_sender, gossip_receiver)) => {
            let mut set = JoinSet::new();
            set.spawn(async move {
                loop {
                    if let Ok(data) = rx.recv().await
                        && let Err(e) = gossip_sender.broadcast(data).await
                    {
                        warn!("Broadcast failed: {e}");
                    }
                }
            });
            set.spawn(async move {
                loop {
                    if let Some(result) = gossip_receiver.next().await
                        && let Err(e) = tx.send(result.map_err(BevyError::from)).await
                    {
                        warn!("Send failed: {e}");
                    }
                }
            });
            set.join_all().await;
        }
        Err(e) => {
            if let Err(e2) = tx.send(Err(e)).await {
                warn!("Send error failed: {e2}");
            }
        }
    }
}
