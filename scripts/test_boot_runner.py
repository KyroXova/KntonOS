"""Exercise boot-result validation with deterministic emulated serial streams."""

import contextlib
import io
import unittest
from unittest.mock import Mock, patch

import boot_runner


class BootRunnerTests(unittest.TestCase):
    def run_stream(self, tokens):
        process = Mock()
        process.stdout = io.StringIO("\n".join(tokens) + "\n")
        process.poll.return_value = None
        with contextlib.ExitStack() as stack:
            for name, value in [
                ("os.path.isfile", True),
                ("os.path.getsize", 123),
                ("os.makedirs", None),
                ("shutil.copy2", None),
                ("find_qemu_executable", "qemu"),
                ("find_ovmf_firmware", "ovmf.fd"),
                ("subprocess.Popen", process),
            ]:
                stack.enter_context(patch("boot_runner." + name, return_value=value))
            stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
            stack.enter_context(contextlib.redirect_stderr(io.StringIO()))
            result = boot_runner.run_boot_test("fixture.efi", timeout_sec=1)
        process.terminate.assert_called_once()
        process.wait.assert_called_once_with(timeout=2)
        return result

    def test_complete_boot_passes(self):
        self.assertEqual(self.run_stream([
            "[KNT-UEFI-START]", "[KNT-KERNEL-INIT]", "[KNT-BOOT-SUCCESS]",
        ]), 0)

    def test_missing_initialization_fails(self):
        for initialization in [[], ["[KNT-UEFI-START]"], ["[KNT-KERNEL-INIT]"]]:
            with self.subTest(initialization=initialization):
                self.assertEqual(self.run_stream(initialization + ["[KNT-BOOT-SUCCESS]"]), 1)

    def test_failure_overrides_success(self):
        for failure in ["[KNT-TEST-FAIL]", "[KNT-PANIC]"]:
            with self.subTest(failure=failure):
                self.assertEqual(self.run_stream([
                    "[KNT-UEFI-START]", "[KNT-KERNEL-INIT]", failure, "[KNT-BOOT-SUCCESS]",
                ]), 1)


if __name__ == "__main__":
    unittest.main()
