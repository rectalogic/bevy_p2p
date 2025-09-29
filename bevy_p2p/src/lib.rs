use std::time::Duration;

use async_channel::TryRecvError;
use bevy::{ecs::error::Result, prelude::*};
use iroh::{Endpoint, NodeId, Watcher, protocol::Router};
pub use iroh_gossip::api::Event as PeerEvent;
use iroh_gossip::{
    api::{GossipReceiver, GossipSender},
    net::Gossip,
    proto::TopicId,
};
use nostr_sdk::prelude::*;
use sha2::Digest;
use tokio::task::JoinSet;

// Custom event kind for iroh peer announcements
// 10000 <= n < 20000 are replaceable https://github.com/nostr-protocol/nips/blob/master/01.md
const NOSTR_PEER_ANNOUNCEMENT_KIND: u16 = 17643;
const NOSTR_ANNOUNCEMENT_INTERVAL: Duration = Duration::from_secs(30);

pub struct Peer2PeerPlugin {
    pub topic: String,
    pub relay_urls: Vec<String>,
}

impl Plugin for Peer2PeerPlugin {
    fn build(&self, app: &mut App) {
        let (send_tx, send_rx) = async_channel::unbounded();
        let (recv_tx, recv_rx) = async_channel::unbounded();
        let topic_id = topic_id(&self.topic);
        let relay_urls = self.relay_urls.clone();
        std::thread::spawn(move || {
            peer_main(topic_id, &relay_urls, send_rx, recv_tx);
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

fn topic_id(s: &str) -> TopicId {
    let mut raw_hash = sha2::Sha512::new();
    raw_hash.update(s.as_bytes());
    TopicId::from_bytes(
        raw_hash.finalize()[..32]
            .try_into()
            .expect("hash topic failed"),
    )
}

async fn peer_setup(
    topic_id: TopicId,
    relay_urls: &[String],
) -> Result<(Router, GossipSender, GossipReceiver)> {
    let nostr_client = Client::new(Keys::generate());
    for relay in relay_urls.iter() {
        nostr_client.add_relay(relay).await?;
    }
    nostr_client.connect().await;
    nostr_client
        .wait_for_connection(Duration::from_secs(5))
        .await;

    let endpoint = Endpoint::builder().discovery_n0().bind().await?;
    // endpoint.node_addr().initialized().await;
    tokio::task::spawn(nostr_announce(nostr_client.clone(), endpoint.node_id()));

    let gossip = Gossip::builder().spawn(endpoint.clone());
    let router = Router::builder(endpoint.clone())
        .accept(iroh_gossip::ALPN, gossip.clone())
        .spawn();

    let topic = gossip.subscribe(topic_id, Vec::new()).await?;
    let (gossip_sender, gossip_receiver) = topic.split();
    tokio::task::spawn(nostr_discover(nostr_client.clone(), gossip_sender.clone()));

    Ok((router, gossip_sender, gossip_receiver))
}

async fn nostr_announce(nostr_client: Client, node_id: NodeId) -> Result<()> {
    let mut interval = tokio::time::interval(NOSTR_ANNOUNCEMENT_INTERVAL);
    let content = serde_json::to_string(&node_id)?;
    loop {
        let builder = EventBuilder::new(Kind::Custom(NOSTR_PEER_ANNOUNCEMENT_KIND), &content);
        //XXX add tags? .tags(tags); -- filter to iroh topic on nostr

        nostr_client.send_event_builder(builder).await?;
        println!("Announced {}", node_id.fmt_short());
        interval.tick().await;
    }
}

async fn nostr_discover(nostr_client: Client, gossip_sender: GossipSender) -> Result<()> {
    let filter = Filter::new()
        .kind(Kind::Custom(NOSTR_PEER_ANNOUNCEMENT_KIND))
        .since(Timestamp::now() - NOSTR_ANNOUNCEMENT_INTERVAL * 2);

    nostr_client.subscribe(filter, None).await?;
    nostr_client
        .handle_notifications(async |notification| {
            if let RelayPoolNotification::Event { event, .. } = notification
                && event.kind == Kind::Custom(NOSTR_PEER_ANNOUNCEMENT_KIND)
                && let Ok(node_id) = serde_json::from_str::<NodeId>(&event.content)
            {
                if let Err(err) = gossip_sender.join_peers(vec![node_id]).await {
                    eprintln!("Failed to join {}: {err}", node_id.fmt_short());
                } else {
                    println!("Joined {}", node_id.fmt_short());
                }
            }
            Ok(false)
        })
        .await?;
    Ok(())
}

#[tokio::main]
async fn peer_main(
    topic_id: TopicId,
    relay_urls: &[String],
    rx: async_channel::Receiver<Vec<u8>>,
    tx: async_channel::Sender<Result<PeerEvent>>,
) {
    match peer_setup(topic_id, relay_urls).await {
        Ok((_router, gossip_sender, mut gossip_receiver)) => {
            let mut set = JoinSet::new();
            set.spawn(async move {
                loop {
                    if let Ok(data) = rx.recv().await
                        && let Err(e) = gossip_sender.broadcast(data.into()).await
                    {
                        warn!("Broadcast failed: {e}");
                    }
                }
            });
            set.spawn(async move {
                //XXX start/stop discovery task when we have neighbors
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
