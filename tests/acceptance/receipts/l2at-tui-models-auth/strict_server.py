import http.server, json, sys
PORT = int(sys.argv[1]); LOG = "/home/dm/pony-agent/tests/acceptance/receipts/l2at-tui-models-auth/a3-server-requests.log"
EXPECT_KEY = "test-dummy-key"
EXPECT_MODEL = "alpha-1"
def log(obj):
    with open(LOG, "a") as f:
        f.write(json.dumps(obj, ensure_ascii=False) + "\n")
class H(http.server.BaseHTTPRequestHandler):
    def _send(self, code, obj):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def _record(self):
        length = int(self.headers.get("Content-Length", 0) or 0)
        body = self.rfile.read(length).decode("utf-8", "replace") if length else ""
        rec = {"path": self.path, "authorization": self.headers.get("Authorization", ""),
               "body": body}
        try:
            rec["model"] = json.loads(body).get("model") if body else None
        except Exception:
            rec["model"] = None
        log(rec)
        return rec
    def do_POST(self):
        rec = self._record()
        p = self.path.split("?")[0].rstrip("/")
        if p.endswith("/chat/completions") or p.endswith("/messages"):
            auth_ok = rec["authorization"] == f"Bearer {EXPECT_KEY}"
            model_ok = rec["model"] == EXPECT_MODEL
            if auth_ok and model_ok:
                # minimal OpenAI-style JSON response (content not asserted)
                self._send(200, {"id": "chatcmpl-1", "object": "chat.completion",
                                 "model": rec["model"], "choices": [
                                     {"index": 0, "finish_reason": "stop",
                                      "message": {"role": "assistant", "content": "ok"}}]})
            else:
                self._send(401, {"error": "unauthorized"})
        else:
            self._send(404)
    def do_GET(self):
        p = self.path.split("?")[0].rstrip("/")
        if p.endswith("/models"):
            log({"path": self.path, "authorization": self.headers.get("Authorization", ""), "body": ""})
            body = json.dumps({"data": [{"id": "alpha-1"}, {"id": "beta-2"}, {"id": "gamma-3"}]}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self._send(404)
    def log_message(self, *a):
        pass
http.server.HTTPServer(("127.0.0.1", PORT), H).serve_forever()
