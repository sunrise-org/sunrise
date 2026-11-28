import unittest

from tools import lab, test_environment_render_parity as environment, test_hud_parity as hud, \
    test_recipe_book_render_parity as recipe_book


def row(text):
    return lab.exact_json(text)


class RecipeBookTests(unittest.TestCase):
    def test_a_blits_uv_compares_at_binary32_width(self):
        vanilla = row('{"type": "blit", "u": 0.0, "v": 0.5, "m": [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]}')
        sunrise = row('{"type": "blit", "u": -0.0, "v": 0.5, "m": [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]}')
        self.assertTrue(vanilla['u'] == sunrise['u'], "Python equality can't tell 0.0 from -0.0")
        self.assertFalse(recipe_book.same(vanilla, sunrise))
        self.assertTrue(recipe_book.same(vanilla, row('{"type": "blit", "u": 0, "v": 0.5, '
                                                     '"m": [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]}')))


    def test_an_integer_field_never_matches_a_float(self):
        vanilla = row('{"t": "text", "runs": [["x", -1, 0]], "x": 3, "y": 4, "color": -1, "shadow": true}')
        regressed = row('{"t": "text", "runs": [["x", -1, 0]], "x": 3, "y": 4, "color": -1.00000001, "shadow": true}')
        in_a_run = row('{"t": "text", "runs": [["x", -1.00000001, 0]], "x": 3, "y": 4, "color": -1, "shadow": true}')
        self.assertEqual(lab.f32_bits(vanilla['color']), lab.f32_bits(regressed['color']),
                         "as binary32 the two colours would be one value")
        self.assertFalse(recipe_book.same(vanilla, regressed))
        self.assertFalse(recipe_book.same(vanilla, in_a_run))
        self.assertTrue(recipe_book.same(vanilla, row('{"t": "text", "runs": [["x", -1, 0]], "x": 3, "y": 4, '
                                                     '"color": -1, "shadow": true}')))


class HudTests(unittest.TestCase):
    def test_an_integer_field_never_matches_a_float(self):
        vanilla = row('{"type": "fill", "x0": 0, "y0": 0, "x1": 4, "y1": 4, "color": -1}')
        regressed = row('{"type": "fill", "x0": 0, "y0": 0, "x1": 4, "y1": 4, "color": -1.00000001}')
        self.assertFalse(hud.same(hud.normal(vanilla), hud.normal(regressed)))

    def test_the_matrix_still_compares_at_binary32_width(self):
        a = row('{"type": "fill", "x0": 0, "y0": 0, "x1": 4, "y1": 4, "color": -1, "matrix": [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]}')
        b = row('{"type": "fill", "x0": 0, "y0": 0, "x1": 4, "y1": 4, "color": -1, "matrix": [1.0, -0.0, 0.0, 1.0, 0.0, 0.0]}')
        self.assertFalse(hud.same(hud.normal(a), hud.normal(b)))
        self.assertTrue(hud.same(hud.normal(a), hud.normal(a)))


class EnvironmentTests(unittest.TestCase):
    def full(self):
        return {key: 0.5 for key in environment.MODELED}

    def test_a_row_without_the_modeled_attributes_fails(self):
        vanilla = [{'case': 'overworld 0 0 0 plains', **self.full()}]
        bad, unmodeled = environment.compare_rows(vanilla, [{'case': 'overworld 0 0 0 plains'}])
        self.assertEqual(set(bad), set(environment.MODELED))
        self.assertEqual(unmodeled, set())

    def test_vanilla_only_attributes_are_reported_not_failed(self):
        vanilla = [{'case': 'c', **self.full(), 'minecraft:visual/water_fog_color': 3}]
        bad, unmodeled = environment.compare_rows(vanilla, [{'case': 'c', **self.full()}])
        self.assertEqual(bad, {})
        self.assertEqual(unmodeled, {'minecraft:visual/water_fog_color'})

    def test_an_attribute_sunrise_prints_outside_modeled_fails(self):
        bad, _ = environment.compare_rows([{'case': 'c', **self.full()}],
                                          [{'case': 'c', **self.full(), 'minecraft:visual/new': 1.0}])
        self.assertEqual(set(bad), {'minecraft:visual/new'})


if __name__ == '__main__':
    unittest.main()
