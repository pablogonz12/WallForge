"""Tests for the rotation picker (no GTK needed)."""
import types
import unittest

from wallforge import rotation


def item(i, kind="scene", broken=""):
    return types.SimpleNamespace(id=str(i), title=f"W{i}", kind=kind, broken=broken, run_dir=f"/tmp/{i}")


class CandidatesTest(unittest.TestCase):
    def test_keeps_playable_wallpapers_in_order(self):
        items = [item(1), item(2), item(3, broken="missing scene.pkg")]
        self.assertEqual([i.id for i in rotation.candidates(items)], ["1", "2"])

    def test_hidden_filter_matches_the_library_view(self):
        items = [item(1), item(2)]
        pool = rotation.candidates(items, hidden=lambda i: i.id == "1")
        self.assertEqual([i.id for i in pool], ["2"])

    def test_only_favourites(self):
        items = [item(1), item(2), item(3)]
        pool = rotation.candidates(items, favourites=["2", "3"], only_favourites=True)
        self.assertEqual([i.id for i in pool], ["2", "3"])
        # favourites are ignored when the option is off
        self.assertEqual(len(rotation.candidates(items, favourites=["2"])), 3)

    def test_web_wallpapers_need_the_player(self):
        items = [item(1), item(2, kind="web")]
        self.assertEqual(len(rotation.candidates(items, web_missing=True)), 1)
        self.assertEqual(len(rotation.candidates(items, web_missing=False)), 2)

    def test_unknown_favourites_are_harmless(self):
        self.assertEqual(rotation.candidates([item(1)], favourites=["9"], only_favourites=True), [])


class NextItemTest(unittest.TestCase):
    def test_empty_pool(self):
        self.assertIsNone(rotation.next_item([]))

    def test_shuffle_never_repeats_what_is_showing(self):
        pool = [item(i) for i in range(6)]
        for _ in range(200):
            self.assertNotIn(rotation.next_item(pool, {"0", "1"}).id, {"0", "1"})

    def test_shuffle_falls_back_to_the_whole_pool(self):
        pool = [item(1), item(2)]
        self.assertIn(rotation.next_item(pool, {"1", "2"}).id, {"1", "2"})

    def test_sequential_follows_library_order_and_wraps(self):
        pool = [item(i) for i in range(4)]
        self.assertEqual(rotation.next_item(pool, {"1"}, shuffle=False).id, "2")
        self.assertEqual(rotation.next_item(pool, {"3"}, shuffle=False).id, "0")
        # an id that is no longer in the pool starts from the top
        self.assertEqual(rotation.next_item(pool, {"99"}, shuffle=False).id, "0")

    def test_sequential_skips_every_current_item(self):
        pool = [item(i) for i in range(4)]
        self.assertEqual(rotation.next_item(pool, {"0", "1"}, shuffle=False).id, "2")


if __name__ == "__main__":
    unittest.main()
