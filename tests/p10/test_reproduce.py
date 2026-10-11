import os
import stat
import sys
import tempfile
import unittest
from pathlib import Path


if os.name == "posix":
    from tools.p10.reproduce import execute_application, sha_file


@unittest.skipUnless(os.name == "posix", "native execution receipt requires POSIX")
class ReproduceExecutionTests(unittest.TestCase):
    def test_execution_receipt_binds_executable_and_health_exchange(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            executable = root / "application.elf"
            executable.write_text(
                "#!" + sys.executable + "\n"
                "import os, signal, socket\n"
                "assert os.read(4, 16) == b'{\\\"grant\\\":true}'\n"
                "def stop(*_): raise SystemExit(0)\n"
                "signal.signal(signal.SIGTERM, stop)\n"
                "with socket.socket() as server:\n"
                "    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)\n"
                "    server.bind(('127.0.0.1', 8080)); server.listen(1)\n"
                "    while True:\n"
                "        client, _ = server.accept()\n"
                "        with client:\n"
                "            client.recv(4096)\n"
                "            client.sendall(b'HTTP/1.1 200 OK\\r\\nContent-Length: 0\\r\\n\\r\\n')\n",
                encoding="utf-8",
            )
            executable.chmod(executable.stat().st_mode | stat.S_IXUSR)
            policy = root / "policy.json"
            policy.write_text('{"grant":true}', encoding="utf-8")

            receipt = execute_application(executable, policy, root / "execution", timeout=3)

            self.assertTrue(receipt["clean_environment"])
            self.assertTrue(receipt["health_check_passed"])
            self.assertEqual(receipt["error"], None)
            self.assertEqual(receipt["executable_sha256"], sha_file(executable))
            self.assertEqual(receipt["exit_code"], 0)
            self.assertEqual(receipt["termination_signal"], None)
            self.assertGreater(receipt["elapsed_ns"], 0)


if __name__ == "__main__":
    unittest.main()
