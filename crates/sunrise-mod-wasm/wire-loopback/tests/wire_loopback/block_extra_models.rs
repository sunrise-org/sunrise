//! `ClientRenderClientHost::set_block_extra_models` from a wasm mod reaches the client with the
//! position and the handles intact, in order, and each named refusal comes back as the native
//! call returns it (#2763).
use super::*;
use mod_api::prelude::*;
use mod_api::{BlockExtraModelsError, ModelError, ModelRef};
use std::cell::RefCell;

/// A client that refuses as the game would for these positions and handles, and records what
/// reached it.
#[derive(Default)]
struct Client {
    calls: RefCell<Vec<([i32; 3], Vec<ModelRef>)>>,
}
impl mod_api::Host for Client {}
mod_api::impl_host_areas!(Client);
impl mod_api::ClientHost for Client {}
mod_api::impl_client_host_areas!(Client, except(client_render));
impl ClientRenderClientHost for Client {
    fn set_block_extra_models(
        &mut self,
        pos: [i32; 3],
        models: &[ModelRef],
    ) -> Result<(), BlockExtraModelsError> {
        self.calls.borrow_mut().push((pos, models.to_vec()));
        if models.len() > mod_api::BLOCK_EXTRA_MODELS_MAX {
            Err(BlockExtraModelsError::TooMany)
        } else if pos[1] > 319 {
            Err(BlockExtraModelsError::NotLoaded)
        } else if models.contains(&model(4, 2)) {
            Err(BlockExtraModelsError::Model(ModelError::StaleModel))
        } else {
            Ok(())
        }
    }
}

fn model(generation: u64, slot: u32) -> ModelRef {
    mod_api::__private::built::model_ref(generation, slot)
}

#[test]
fn a_wasm_mods_extra_models_reach_the_client_and_get_each_refusal_by_name() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let mut client = Client::default();
    let mut frame = boundary::Frame::default();
    frame.set_client_host_mut(&mut client);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let cx = RefCell::new(cx);
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        // SAFETY: client and its frame remain live for this synchronous transport.
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    let mut guest = guest::Guest;
    let parts = [model(5, 2), model(5, 9), model(5, 2)];
    assert_eq!(guest.set_block_extra_models([3, -60, -7], &parts), Ok(()));
    assert_eq!(guest.set_block_extra_models([3, -60, -7], &[]), Ok(()));
    let over = vec![model(5, 1); mod_api::BLOCK_EXTRA_MODELS_MAX + 1];
    assert_eq!(
        guest.set_block_extra_models([0, 64, 0], &over),
        Err(BlockExtraModelsError::TooMany)
    );
    assert_eq!(
        guest.set_block_extra_models([0, 400, 0], &parts),
        Err(BlockExtraModelsError::NotLoaded)
    );
    assert_eq!(
        guest.set_block_extra_models([0, 64, 0], &[model(4, 2)]),
        Err(BlockExtraModelsError::Model(ModelError::StaleModel))
    );
    guest::set_transport(None);
    let reached = vec![
        ([3, -60, -7], parts.to_vec()),
        ([3, -60, -7], Vec::new()),
        ([0, 64, 0], over),
        ([0, 400, 0], parts.to_vec()),
        ([0, 64, 0], vec![model(4, 2)]),
    ];
    assert_eq!(*client.calls.borrow(), reached);
}
