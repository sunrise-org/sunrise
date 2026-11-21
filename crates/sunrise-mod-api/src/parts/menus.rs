//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;
#[cfg(test)]
#[path = "menus/book_tests.rs"]
mod book_tests;
#[cfg(test)]
#[path = "menus/screen_tests.rs"]
mod screen_tests;

impl MenusRegistrar for PartRegistrar {
    fn menu_recipe_book(
        &mut self,
        menu: &str,
        book: &crate::RecipeBookTypeKey,
        recipe_type: &str,
        layout: crate::RecipeBookLayout,
        screen: crate::Screen,
    ) -> Result<(), String> {
        let result: Result<(), String> = (|| {
            if mod_vocab::__private::parts_installation::parts_installed() {
                return Err("registry is frozen".into());
            }
            let menu_id =
                crate::Identifier::parse(menu).map_err(|e| format!("invalid menu: {e:?}"))?;
            if !menu.contains(':') || !owns_namespace(self.mod_id, menu_id.namespace()) {
                return Err(
                    "menu namespace must be explicit and owned by the registering mod".into(),
                );
            }
            if !self.recipe_book_types.contains_key(book.as_str()) {
                return Err(format!("unknown recipe book {}", book.as_str()));
            }
            if self.menu_recipe_books.contains_key(menu) {
                return Err("duplicate menu recipe book".into());
            }
            crate::Identifier::parse(recipe_type)
                .map_err(|e| format!("invalid recipe type: {e:?}"))?;
            if !recipe_type.contains(':') {
                return Err("explicit recipe type namespace required".into());
            }
            layout.validate(256)?;
            layout.validate_screen(&screen)?;
            if screen.recipe_book.as_ref() != Some(book) {
                return Err("screen book differs from menu book".into());
            }
            if let Some(other) = self
                .parts
                .menus
                .get(menu)
                .into_iter()
                .flatten()
                .find(|other| other.mod_id != self.mod_id && lays_out_screen(&*other.part))
            {
                return Err(format!(
                    "recipe-book screen conflicts with {} of mod {}",
                    other.type_name, other.mod_id
                ));
            }
            self.menu_recipe_books.insert(
                menu.into(),
                MenuBookDeclaration {
                    owner: self.mod_id,
                    book: book.clone(),
                    recipe_type: recipe_type.into(),
                    layout,
                    screen,
                },
            );
            Ok(())
        })();
        result.map_err(|reason| {
            let message = format!("mod {} menu {menu:?}: {reason}", self.mod_id);
            self.refused.push(message.clone());
            message
        })
    }
    fn attach_menu_part(&mut self, menu: &str, kind: &'static str, part: Arc<dyn MenuPart>) {
        if lays_out_screen(&*part) {
            if let Some(book) = self.menu_recipe_books.get(menu)
                && book.owner != self.mod_id
            {
                self.refused.push(format!(
                    "{kind} of mod {} conflicts with the recipe-book screen on menu {menu:?} owned by mod {}",
                    self.mod_id, book.owner
                ));
                return;
            }
            // One part owns a screen: a second would split its layout, drawing and clicks.
            if let Some(other) = self
                .parts
                .menus
                .get(menu)
                .into_iter()
                .flatten()
                .find(|other| {
                    lays_out_screen(&*other.part) && screens_shared(menu, other.mod_id, self.mod_id)
                })
            {
                self.refused.push(format!(
                    "{kind} of {} and {} of {} both lay out menu type {menu}'s screen",
                    self.mod_id, other.type_name, other.mod_id
                ));
                return;
            }
        }
        self.attach(|p| &mut p.menus, kind, menu, part);
    }
}
