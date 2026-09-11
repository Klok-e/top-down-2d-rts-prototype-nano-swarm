import json
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import unittest
from concurrent.futures import ThreadPoolExecutor


SCRIPT = Path(__file__).parents[1] / "scripts" / "nano_swarm_control.py"


class ExecutionSnapshotClientTests(unittest.TestCase):
    def request_state(self, *flags):
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
                         "--timeout", "3", "state", "--cell-limit", "1", *flags],
                        capture_output=True, text=True, timeout=5,
                    )
                    self.assertEqual(process.returncode, 0, process.stderr)
                    self.assertTrue(json.loads(process.stdout)["ok"])
                    return received.result(timeout=3)

    def test_details_flag_reaches_the_state_request(self):
        request = self.request_state("--details")
        self.assertEqual(request["method"], "state.get")
        self.assertEqual(request["params"], {
            "cell_offset": 0, "cell_limit": 1, "details": True,
        })

    def test_default_state_does_not_request_execution_details(self):
        request = self.request_state()
        self.assertEqual(request["method"], "state.get")
        self.assertNotIn("details", request["params"])


if __name__ == "__main__":
    unittest.main()
