//! `Registrar::tag_key` from a wasm mod's `register` crosses the generated boundary and is
//! answered by the host's registrar, so the guest holds the key the host resolves for the same
//! name, whether the name is bare or namespaced.
use super::*;
use mod_api::Registrar;
use std::cell::RefCell;

thread_local! {
    /// The names this file's registration asks for, and the keys it got back.
    static ASKED: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
    static GOT: RefCell<Vec<mod_api::TagKey>> = const { RefCell::new(Vec::new()) };
}

pub(super) struct TagKeyRegistrationHooks;
impl TagKeyRegistrationHooks {
    pub(super) fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        for name in ASKED.take() {
            GOT.with_borrow_mut(|got| got.push(registrar.tag_key(name)));
        }
    }
}

/// Install the loopback mod with this file's registration asking for `names`.
fn install_asking(names: Vec<&'static str>) -> Result<Vec<mod_api::TagKey>, Vec<String>> {
    ASKED.set(names);
    GOT.take();
    let mods = [mod_api::ModEntry::new(
        "wire_test",
        Box::new(WasmMod {
            group: 0,
            subscribed: &[],
            listeners: &[],
            corrupt: false,
        }),
    )];
    mod_api::__private::install_parts(
        &mods,
        &mod_api::__private::part_tests::rule_schema(),
        |_| false,
    )
    .map(|_| GOT.take())
}

#[test]
fn wire_loopback_a_guest_tag_key_is_the_key_the_host_resolves() {
    let keys = install_asking(vec!["wire_test:planks", "logs"]).unwrap();
    for (key, name) in keys.iter().zip(["wire_test:planks", "minecraft:logs"]) {
        let identifier = mod_api::Identifier::parse(name).unwrap();
        assert_eq!(
            Some(*key),
            mod_api::__private::tag_key(&identifier),
            "{name}"
        );
        assert_eq!(
            mod_api::__private::with_tag_name(*key, |n| n.map(str::to_owned)),
            Some(name.to_owned())
        );
    }
    assert_eq!(keys.len(), 2);
    assert_ne!(keys[0], keys[1]);
}
