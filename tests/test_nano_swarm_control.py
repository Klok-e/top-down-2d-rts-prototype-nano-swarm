import json
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import unittest
from concurrent.futures import ThreadPoolExecutor


SCRIPT = Path(__file__).parents[1] / "scripts" / "nano_swarm_control.py"


class ControlClientTests(unittest.TestCase):
    def exchange(self, *command):
        with tempfile.TemporaryDirectory(prefix="ns-client-") as directory:
            path = Path(directory) / "control.sock"
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
                listener.bind(str(path))
                listener.listen(1)
                listener.settimeout(3)

                def exchange():
                    connection, _ = listener.accept()
                    with connection:
                        connection.settimeout(3)
                        with connection.makefile("rb") as incoming:
                            request = json.loads(incoming.readline())
                        response = {"id": request["id"], "ok": True, "result": {}}
                        connection.sendall(json.dumps(response).encode() + b"\n")
                    return request

                with ThreadPoolExecutor(max_workers=1) as executor:
                    received = executor.submit(exchange)
                    process = subprocess.run(
                        [sys.executable, str(SCRIPT), "--socket", str(path),
                         "--timeout", "3", *command],
                        capture_output=True, text=True, timeout=5,
                    )
                    self.assertEqual(process.returncode, 0, process.stderr)
                    self.assertTrue(json.loads(process.stdout)["ok"])
                    return received.result(timeout=3)

    def test_details_flag_reaches_the_state_request(self):
        request = self.exchange("state", "--cell-limit", "1", "--details")
        self.assertEqual(request["method"], "state.get")
        self.assertEqual(request["params"], {
            "cell_offset": 0, "cell_limit": 1, "details": True,
        })

    def test_default_state_does_not_request_execution_details(self):
        request = self.exchange("state", "--cell-limit", "1")
        self.assertEqual(request["method"], "state.get")
        self.assertNotIn("details", request["params"])

    def test_perf_hitches_sends_an_optional_episode_cursor(self):
        request = self.exchange("perf-hitches")
        self.assertEqual(request["method"], "perf.hitches")
        self.assertNotIn("params", request)

        request = self.exchange("perf-hitches", "--after-id", "27")
        self.assertEqual(request["method"], "perf.hitches")
        self.assertEqual(request["params"], {"after_id": 27})


if __name__ == "__main__":
    unittest.main()
