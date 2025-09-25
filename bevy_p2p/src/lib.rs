use bevy::{
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, block_on, futures_lite::future},
};
use distributed_topic_tracker::{
    AutoDiscoveryGossip, GossipReceiver, GossipSender, RecordPublisher, TopicId,
};
use iroh::{Endpoint, protocol::Router};
use iroh_gossip::{api::Event, net::Gossip};

pub struct Peer2PeerPlugin {
    pub topic: String,
    pub initial_secret: Vec<u8>,
}

impl Plugin for Peer2PeerPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(SetupConfig {
            topic_id: TopicId::new(self.topic.clone()),
            initial_secret: self.initial_secret.clone(),
        })
        .init_resource::<PeerReceiver>()
        .add_systems(Startup, setup)
        .add_systems(Update, handle_setup_task);
    }
}

#[derive(Resource)]
struct SetupConfig {
    topic_id: TopicId,
    initial_secret: Vec<u8>,
}

#[derive(Component)]
struct SetupTask(Task<Result<(Router, GossipSender, GossipReceiver)>>);

#[derive(Resource)]
#[allow(dead_code)]
struct IrohRouter(Router);

#[derive(Resource, Clone)]
pub struct PeerSender(async_channel::Sender<Vec<u8>>);

impl PeerSender {
    pub fn send(&self, data: Vec<u8>) -> Result<()> {
        self.0.send_blocking(data)?;
        Ok(())
    }
}

#[derive(Resource, Default, Clone)]
pub struct PeerReceiver(Option<GossipReceiver>);

impl PeerReceiver {
    pub fn recv(&self) -> Option<Result<Event>> {
        if let Some(ref receiver) = self.0 {
            if let Some(Some(result)) = block_on(future::poll_once(receiver.next())) {
                match result {
                    Err(err) => Some(Err(err.into())),
                    Ok(event) => Some(Ok(event)),
                }
            } else {
                None
            }
        } else {
            None
        }
    }
}

fn setup(mut commands: Commands, config: Res<SetupConfig>) {
    let topic_id = config.topic_id.clone();
    let initial_secret = config.initial_secret.clone();
    let task = AsyncComputeTaskPool::get().spawn(async move {
        let endpoint = Endpoint::builder().discovery_n0().bind().await?;
        let gossip = Gossip::builder().spawn(endpoint.clone());
        let router = iroh::protocol::Router::builder(endpoint.clone())
            .accept(iroh_gossip::ALPN, gossip.clone())
            .spawn(); //XXX internally this spawns a tokio task
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
    });
    commands.spawn(SetupTask(task));
}

fn handle_setup_task(
    mut commands: Commands,
    setup_task: Single<(Entity, &mut SetupTask)>,
    mut peer_receiver: ResMut<PeerReceiver>,
) {
    let (entity, mut task) = setup_task.into_inner();

    if let Some(result) = block_on(future::poll_once(&mut task.0)) {
        match result {
            Ok((router, gossip_sender, gossip_receiver)) => {
                commands.insert_resource(IrohRouter(router));
                peer_receiver.0 = Some(gossip_receiver);
                let (tx, rx) = async_channel::unbounded();
                commands.insert_resource(PeerSender(tx));
                AsyncComputeTaskPool::get()
                    .spawn::<Result<()>>(async move {
                        loop {
                            gossip_sender.broadcast(rx.recv().await?).await?;
                        }
                    })
                    .detach();
                commands.entity(entity).despawn();
            }
            Err(e) => warn!("Setup task failed: {e}"),
        }
    }
}
