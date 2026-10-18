import http.server, json, sys
PORT = int(sys.argv[1])
STATE = {"count": 3, "fail": False}
class H(http.server.BaseHTTPRequestHandler):
    def _send(self, code, body=b""):
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def do_GET(self):
        p = self.path.split("?")[0].rstrip("/")
        if p.endswith("/switch"):
            STATE["count"] = 2
            self._send(200, b'{"ok":true}')
        elif p.endswith("/fail500"):
            STATE["fail"] = True
            self._send(200, b'{"ok":true}')
        elif p.endswith("/models"):
            if STATE["fail"]:
                self._send(500, b'{"error":"internal boom"}')
                return
            ids = ["alpha-1", "beta-2", "gamma-3"][:STATE["count"]]
            body = json.dumps({"data": [{"id": i} for i in ids]}).encode()
            self._send(200, body)
        else:
            self._send(404)
    def log_message(self, *a):
        pass
http.server.HTTPServer(("127.0.0.1", PORT), H).serve_forever()
