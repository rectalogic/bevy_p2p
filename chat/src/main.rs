use bevy::prelude::*;
use bevy_p2p::{Peer2PeerPlugin, PeerEvent, PeerReceiver, PeerSender};

fn main() {
    let mut app = App::new();
    app.add_plugins((
        DefaultPlugins,
        Peer2PeerPlugin {
            topic: "bevy-topic".into(),
            initial_secret: b"bevy-secret".to_vec(),
        },
    ))
    .add_systems(Update, update)
    .add_observer(click_send)
    .run();
}

fn update(receiver: Res<PeerReceiver>) -> Result<()> {
    if let Some(Ok(event)) = receiver.try_recv() {
        dbg!(&event);
    }
    Ok(())
}

fn click_send(click: Trigger<Pointer<Click>>, sender: Res<PeerSender>) -> Result<()> {
    info!("click send");
    let data = b"welcome ".to_vec();
    sender.send(data)?;
    Ok(())
}
