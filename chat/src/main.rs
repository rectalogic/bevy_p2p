use bevy::prelude::*;
use bevy_p2p::{Peer2PeerPlugin, PeerReceiver, PeerSender};

fn main() {
    let mut app = App::new();
    app.add_plugins((
        DefaultPlugins,
        Peer2PeerPlugin {
            topic: "bevy-topic".into(),
            relay_urls: vec!["wss://relay.damus.io".into(), "wss://nos.lol".into()],
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

fn click_send(_click: Trigger<Pointer<Click>>, sender: Res<PeerSender>) -> Result<()> {
    info!("click send");
    let data = b"welcome ".to_vec();
    sender.send(data)?;
    Ok(())
}
