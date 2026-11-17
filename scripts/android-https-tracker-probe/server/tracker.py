"""Minimal HTTPS BitTorrent tracker for the Android probe. Logs every request."""
import http.server, ssl, sys, threading, time
from urllib.parse import urlparse, parse_qs

PKI, LOG = sys.argv[1], sys.argv[2]
lock = threading.Lock()

def log(line):
    with lock, open(LOG, "a") as f:
        f.write(f"{time.strftime('%H:%M:%S')} {line}\n")

def handler(port):
    class H(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            q = parse_qs(urlparse(self.path).query, keep_blank_values=True, encoding="latin-1")
            ih = q.get("info_hash", [""])[0].encode("latin-1").hex() if "info_hash" in q else "-"
            log(f"port={port} GET {urlparse(self.path).path} info_hash={ih} ua={self.headers.get('User-Agent')}")
            if self.path.startswith("/probe.crl"):
                import os
                crl = f"{PKI}/probe.crl"
                if not os.path.exists(f"{PKI}/serve-crl"):
                    self.send_response(404); self.end_headers(); return
                data = open(crl, "rb").read()
                self.send_response(200); self.send_header("Content-Type", "application/pkix-crl")
                self.send_header("Content-Length", str(len(data))); self.end_headers(); self.wfile.write(data); return
            body = b"d8:intervali1800e5:peers0:e"
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers(); self.wfile.write(body)
        def log_message(self, *a): pass
    return H

def serve(port, leaf):
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", port), handler(port))
    if leaf:
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ctx.load_cert_chain(f"{PKI}/{leaf}.crt", f"{PKI}/{leaf}.key")
        srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
    threading.Thread(target=srv.serve_forever, daemon=True).start()

serve(8080, None)          # CRL on an unlisted host (10.0.2.2): the app's policy blocks it
serve(80, None)            # CRL on a listed CA hostname (r10.c.lencr.org via emulator /etc/hosts)
serve(8443, "crl")         # trusted chain, leaf with an HTTP CRL distribution point
serve(8445, "norev")       # trusted chain, leaf without revocation info
serve(8444, "rogue_leaf")  # chain from a root the device does not trust
log("listening")
while True: time.sleep(3600)
