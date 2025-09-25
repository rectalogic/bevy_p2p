use bevy::prelude::*;
use bevy_p2p::{Peer2PeerPlugin, PeerEvent, PeerReceiver, PeerSender};

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

fn update(receiver: Res<PeerReceiver>, sender: Res<PeerSender>) -> Result<()> {
    if let Some(Ok(event)) = receiver.recv() {
        dbg!(&event);
        if let PeerEvent::NeighborUp(node_id) = event {
            let mut data = b"welcome ".to_vec();
            data.append(&mut node_id.to_vec());
            sender.send(data)?;
        }
    }
    Ok(())
}
