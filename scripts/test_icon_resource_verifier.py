import importlib.util
from pathlib import Path
import struct
import unittest

spec = importlib.util.spec_from_file_location("identity", Path(__file__).with_name("verify_geometric_identity.py"))
identity = importlib.util.module_from_spec(spec)
spec.loader.exec_module(identity)


class IconGroupProof(unittest.TestCase):
    def setUp(self):
        self.images = {size: bytes([i])*size for i, size in enumerate([16, 24, 32, 48, 64, 128, 256], 1)}
        self.resources = {i: self.images[size] for i, size in enumerate(self.images, 1)}
        self.group = bytearray(struct.pack('<HHH', 0, 1, 7))
        for i, size in enumerate(self.images, 1):
            self.group.extend(struct.pack('<BBBBHHIH', size % 256, size % 256, 0, 0, 1, 32, len(self.images[size]), i))

    def test_complete_group(self):
        self.assertEqual(identity.verify_group(self.group, self.images, self.resources.__getitem__), list(self.images))

    def test_seven_valid_small_icons_do_not_prove_seven_sizes(self):
        repeated = struct.pack('<HHH', 0, 1, 7)+b''.join(struct.pack('<BBBBHHIH', 16, 16, 0, 0, 1, 32, len(self.images[16]), i) for i in range(1, 8))
        with self.assertRaisesRegex(AssertionError, 'unique embedded sizes'):
            identity.verify_group(repeated, self.images, lambda _: self.images[16])

    def test_wrong_metadata_and_lengths_fail(self):
        for offset, value in [(7, 32), (8, 1), (9, 1), (10, 2), (12, 24), (14, 1)]:
            with self.subTest(offset=offset):
                corrupt = self.group.copy()
                corrupt[offset] = value
                with self.assertRaises(AssertionError):
                    identity.verify_group(corrupt, self.images, self.resources.__getitem__)


if __name__ == '__main__':
    unittest.main()
