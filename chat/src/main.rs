use bevy::prelude::*;
use bevy_p2p::{Peer2PeerPlugin, PeerReceiver};

#[tokio::main]
async fn main() {
    let mut app = App::new();
    app.add_plugins((
        DefaultPlugins,
        Peer2PeerPlugin {
            topic: "bevy-topic".into(),
            initial_secret: b"bevy-secret".to_vec(),
        },
    ))
    .add_systems(Update, update);
    app.run();
}

fn update(receiver: Res<PeerReceiver>) {
    if let Some(Ok(event)) = receiver.recv() {
        dbg!(event);
    }
}
