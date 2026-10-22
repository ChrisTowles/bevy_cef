//! Drains CEF's `OnDraggableRegionsChanged` callbacks: aurora's webviews are not dragged by their
//! `-webkit-app-region: drag` areas, but the channel each browser is created with must not fill.

use async_channel::Receiver;
use bevy::prelude::*;
use bevy_cef_core::prelude::{DraggableRegion, DraggableRegionSenderInner};

pub struct DragPlugin;

impl Plugin for DragPlugin {
    fn build(&self, app: &mut App) {
        let (tx, rx) = async_channel::unbounded();
        app.insert_resource(DraggableRegionSender(tx))
            .insert_resource(DraggableRegionReceiver(rx))
            .add_systems(PreUpdate, drain_drag_regions);
    }
}

#[derive(Resource, Debug, Deref)]
pub(crate) struct DraggableRegionSender(pub(crate) DraggableRegionSenderInner);

#[derive(Resource, Debug)]
pub(crate) struct DraggableRegionReceiver(pub(crate) Receiver<(Entity, Vec<DraggableRegion>)>);

fn drain_drag_regions(receiver: Res<DraggableRegionReceiver>) {
    while receiver.0.try_recv().is_ok() {}
}
