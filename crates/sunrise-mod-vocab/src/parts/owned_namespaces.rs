// exports: none (reached inside the crate only)
use crate::{
    __private::{DeclarationKind, DeclarationRefusal},
    Answered, DeclarationError,
};

/// The package inventory carries the same namespace set as content files and
/// the package loader's collision fence. A package always owns its own ID.
pub fn owns_namespace(owner: &str, namespace: &str) -> bool {
    owner == namespace
        || crate::LoadedMods::new()
            .get(owner)
            .is_some_and(|package| package.namespaces().any(|owned| owned == namespace))
}

/// `name` with its namespace: a bare name is in mod `owner`'s own, for a declared id and
/// for the block and item references a declaration names (`block_item`, `pick_item`,
/// `wall_block`, `base_block`, `crafting_remainder`, `Item::block`) alike.
pub fn qualified(owner: &str, name: &str) -> String {
    if name.contains(':') {
        name.to_owned()
    } else {
        format!("{owner}:{name}")
    }
}

/// Validate and qualify a declaration using the package namespace inventory.
pub fn owned_declaration_id(owner: &str, name: &str) -> Result<String, String> {
    validate_owned_id(owner, qualified(owner, name))
        .map_err(|error| format!("mod {owner} declares {error}"))
}

fn validate_owned_id(owner: &str, id: String) -> Result<String, DeclarationError> {
    let Some(parsed) = crate::Identifier::parse_namespaced(&id) else {
        return Err(DeclarationError::InvalidId { id });
    };
    let namespace = parsed.namespace();
    if !owns_namespace(owner, namespace) {
        return Err(DeclarationError::UnownedNamespace {
            id,
            namespace: namespace.to_owned(),
        });
    }
    Ok(id)
}

/// A declaration a mod's `blocks` or `items` hook adds: its id, and the names of other
/// content it references.
pub(crate) trait Declaration {
    const KIND: DeclarationKind;

    /// The declared id and its optional references.
    fn names(&mut self) -> (&mut String, impl IntoIterator<Item = &mut Option<String>>);
}

impl Declaration for crate::Block {
    const KIND: DeclarationKind = DeclarationKind::Block;

    fn names(&mut self) -> (&mut String, impl IntoIterator<Item = &mut Option<String>>) {
        (
            &mut self.id,
            [
                &mut self.block_item,
                &mut self.pick_item,
                &mut self.wall_block,
                &mut self.base_block,
            ],
        )
    }
}

impl Declaration for crate::Item {
    const KIND: DeclarationKind = DeclarationKind::Item;

    fn names(&mut self) -> (&mut String, impl IntoIterator<Item = &mut Option<String>>) {
        (
            &mut self.id,
            [&mut self.crafting_remainder, &mut self.block],
        )
    }
}

/// Validate at the call site and keep the dispatch owner with the declaration.
pub(crate) fn submit_declaration<D: Declaration>(
    context: &crate::RegistrationContext,
    mod_id: &'static str,
    submit: &mut dyn FnMut(Answered<D>) -> Result<(), DeclarationError>,
    mut declaration: D,
) -> Result<(), DeclarationError> {
    if !crate::registration_open() {
        let error = DeclarationError::RegistrationClosed {
            id: declaration.names().0.clone(),
        };
        log::warn!("mod {mod_id} {:?} declaration refused: {error}", D::KIND);
        return Err(error);
    }
    let result = (|| {
        let (id, references) = declaration.names();
        *id = validate_owned_id(mod_id, qualified(mod_id, id))?;
        for reference in references.into_iter().flatten() {
            *reference = qualified(mod_id, reference);
        }
        submit(Answered {
            answer: declaration,
            mod_id,
        })
    })();
    result.inspect_err(|error| {
        context
            .declaration_refusals
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(DeclarationRefusal::new(mod_id, D::KIND, error.clone()));
    })
}

/// The engine consumes block/item declaration ownership refusals before loading
/// resources.
// hidden: engine admission plumbing, re-exported through __private for the loader.
#[doc(hidden)]
pub fn take_declaration_refusals(context: &crate::RegistrationContext) -> Vec<DeclarationRefusal> {
    std::mem::take(
        &mut *context
            .declaration_refusals
            .lock()
            .unwrap_or_else(|p| p.into_inner()),
    )
}
