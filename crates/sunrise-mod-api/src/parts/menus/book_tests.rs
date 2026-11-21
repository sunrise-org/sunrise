use super::*;
fn roles() -> crate::RecipeBookLayout {
    crate::RecipeBookLayout::Furnace {
        input: 0,
        fuel: 1,
        result: 2,
    }
}
#[test]
fn registration_refuses_unknown_books_duplicate_menus_and_overlapping_roles() {
    let mut registrar = PartRegistrar::new("demo");
    let book = registrar.recipe_book_type("demo:book").unwrap();
    registrar
        .register_menu_screen(
            "demo:pot",
            Arc::new(|_: &dyn ClientHost, _: crate::MenuScreenOpen<'_>| None),
        )
        .unwrap();
    assert!(
        registrar
            .menu_recipe_book("demo:pot", &book, "demo:cooking", roles())
            .is_ok()
    );
    assert!(
        registrar
            .menu_recipe_book("demo:pot", &book, "demo:cooking", roles())
            .is_err()
    );
    assert!(
        registrar
            .menu_recipe_book("other:pot", &book, "demo:cooking", roles())
            .is_err()
    );
    assert!(
        registrar
            .menu_recipe_book(
                "demo:bad",
                &book,
                "demo:cooking",
                crate::RecipeBookLayout::Stonecutter {
                    input: 0,
                    result: 0
                },
            )
            .is_err()
    );
    let unknown = mod_vocab::__private::parts_installation::recipe_book_type("demo:unknown".into());
    assert!(
        registrar
            .menu_recipe_book("demo:unknown", &unknown, "demo:cooking", roles())
            .is_err()
    );
    assert_eq!(registrar.menu_recipe_books.len(), 1);
    assert_eq!(registrar.refused.len(), 4);
    let binding = &registrar.menu_recipe_books["demo:pot"];
    assert_eq!(binding.owner, "demo");
    assert_eq!(binding.book, book);
    assert_eq!(binding.recipe_type, "demo:cooking");
    assert_eq!(binding.layout, roles());
}
