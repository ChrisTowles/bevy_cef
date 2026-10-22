//! The display path for aurora: a webview paints into an `Image` (its `WebviewTextureTarget`);
//! an `ImageNode` webview shows it and takes pointer input from picking hits, so any pointer
//! bevy_picking drives (the window mouse, a world-space panel's ray, an XR laser) reaches the page.

use crate::common::{WebviewSize, WebviewSource, WebviewTextureTarget};
use crate::webview::scroll_delta;
use bevy::asset::{AssetEventSystems, AssetMut, RenderAssetUsages};
use bevy::prelude::*;
use bevy::ui::UiSystems;
use bevy_cef_core::prelude::*;
use wgpu_types::{Extent3d, TextureDimension, TextureFormat};

pub struct ImageWebviewPlugin;

impl Plugin for ImageWebviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<RenderTextureMessage>()
            .add_systems(Update, (send_render_textures, setup_image_webviews))
            .add_systems(
                PostUpdate,
                (
                    paint_texture_targets
                        .run_if(on_message::<RenderTextureMessage>)
                        .before(AssetEventSystems),
                    size_image_webviews.after(UiSystems::Layout),
                ),
            );
    }
}

fn send_render_textures(mut ew: MessageWriter<RenderTextureMessage>, browsers: NonSend<Browsers>) {
    for texture in browsers.try_receive_textures() {
        ew.write(texture);
    }
}

/// Copies a CPU `OnPaint` frame into an `Image`, reallocating it when the frame size differs.
/// Returns `true` when the image was reallocated.
pub(crate) fn update_webview_image(
    texture: &RenderTextureMessage,
    mut image: AssetMut<'_, Image>,
) -> bool {
    let expected_len = (texture.width * texture.height * 4) as usize;
    let dimensions_match = image.texture_descriptor.size.width == texture.width
        && image.texture_descriptor.size.height == texture.height;
    if let Some(data) = image.data.as_mut()
        && dimensions_match
        && data.len() == expected_len
    {
        data.copy_from_slice(&texture.buffer);
        false
    } else {
        *image = Image::new(
            Extent3d {
                width: texture.width,
                height: texture.height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            texture.buffer.clone(),
            TextureFormat::Bgra8UnormSrgb,
            RenderAssetUsages::all(),
        );
        true
    }
}

/// Each page frame into its webview's target image. Popups (a `<select>` list) are not
/// composited yet: only the view is shown.
fn paint_texture_targets(
    mut er: MessageReader<RenderTextureMessage>,
    mut images: ResMut<Assets<Image>>,
    targets: Query<&WebviewTextureTarget>,
) {
    for texture in er.read() {
        if texture.ty != RenderPaintElementType::View {
            continue;
        }
        let Ok(target) = targets.get(texture.webview) else {
            continue;
        };
        if let Some(image) = images.get_mut(target.0.id()) {
            update_webview_image(texture, image);
        }
    }
}

/// A new `ImageNode` webview gets a target image (shown by its node) and its pointer observers.
fn setup_image_webviews(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut webviews: Query<
        (Entity, &mut ImageNode, Option<&WebviewTextureTarget>),
        Added<WebviewSource>,
    >,
) {
    for (entity, mut node, target) in &mut webviews {
        let target = match target {
            Some(target) => target.0.clone(),
            None => {
                let handle = images.add(Image::default());
                commands
                    .entity(entity)
                    .insert(WebviewTextureTarget(handle.clone()));
                handle
            }
        };
        node.image = target;
        commands
            .entity(entity)
            .observe(on_pointer_move)
            .observe(on_pointer_press)
            .observe(on_pointer_release)
            .observe(on_pointer_scroll)
            .observe(on_pointer_out);
    }
}

/// The page's logical size follows its node's laid-out size.
fn size_image_webviews(
    mut webviews: Query<
        (&ComputedNode, &mut WebviewSize),
        (With<WebviewSource>, With<ImageNode>, Changed<ComputedNode>),
    >,
) {
    for (computed, mut size) in &mut webviews {
        let logical = computed.size() * computed.inverse_scale_factor();
        if logical.x >= 1.0 && logical.y >= 1.0 {
            size.set_if_neq(WebviewSize(logical));
        }
    }
}

/// A picking hit's node-relative position ((-0.5, -0.5) top left) in page DIPs.
fn page_pos(hit: &bevy::picking::backend::HitData, size: &WebviewSize) -> Option<Vec2> {
    let p = hit.position?.truncate();
    Some((p + Vec2::splat(0.5)) * size.0)
}

/// Where the pointer last was over the page (scroll events carry no hit).
#[derive(Component, Default)]
struct PagePointer(Vec2);

fn on_pointer_move(
    trigger: On<PointerMove>,
    mut commands: Commands,
    input: Res<ButtonInput<MouseButton>>,
    browsers: NonSend<Browsers>,
    sizes: Query<&WebviewSize>,
) {
    let Ok(size) = sizes.get(trigger.entity) else {
        return;
    };
    if let Some(pos) = page_pos(&trigger.hit, size) {
        browsers.send_mouse_move(&trigger.entity, input.get_pressed(), pos, false);
        commands.entity(trigger.entity).insert(PagePointer(pos));
    }
}

fn on_pointer_press(
    trigger: On<PointerPress>,
    browsers: NonSend<Browsers>,
    sizes: Query<&WebviewSize>,
) {
    let Ok(size) = sizes.get(trigger.entity) else {
        return;
    };
    if let Some(pos) = page_pos(&trigger.hit, size) {
        browsers.send_mouse_click(&trigger.entity, pos, trigger.button, false);
    }
}

fn on_pointer_release(
    trigger: On<PointerRelease>,
    browsers: NonSend<Browsers>,
    sizes: Query<&WebviewSize>,
) {
    let Ok(size) = sizes.get(trigger.entity) else {
        return;
    };
    if let Some(pos) = page_pos(&trigger.hit, size) {
        browsers.send_mouse_click(&trigger.entity, pos, trigger.button, true);
    }
}

fn on_pointer_scroll(
    trigger: On<PointerScroll>,
    browsers: NonSend<Browsers>,
    pointers: Query<&PagePointer>,
) {
    let pos = pointers.get(trigger.entity).map_or(Vec2::ZERO, |p| p.0);
    let delta = scroll_delta(trigger.unit, trigger.x, trigger.y);
    browsers.send_mouse_wheel(&trigger.entity, pos, delta);
}

fn on_pointer_out(
    trigger: On<PointerOut>,
    browsers: NonSend<Browsers>,
    pointers: Query<&PagePointer>,
) {
    let pos = pointers.get(trigger.entity).map_or(Vec2::ZERO, |p| p.0);
    browsers.send_mouse_move(&trigger.entity, std::iter::empty(), pos, true);
}
