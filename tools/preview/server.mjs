// Design preview: serves ui/ with a mock host bridge so pages can be rendered in headless Edge.
// usage: node tools/preview/server.mjs [port]   then open http://127.0.0.1:port/chrome/index.html?dark=1
import http from 'http';
import fs from 'fs';
import path from 'path';
import url from 'url';

const root = path.resolve(path.dirname(url.fileURLToPath(import.meta.url)), '../../ui');
const port = +process.argv[2] || 47990;
const mock = fs.readFileSync(path.join(path.dirname(url.fileURLToPath(import.meta.url)), 'mock.js'), 'utf8');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.png': 'image/png', '.svg': 'image/svg+xml', '.json': 'application/json' };

http.createServer((req, res) => {
  const u = new URL(req.url, 'http://x');
  let p = decodeURIComponent(u.pathname);
  if (p.endsWith('/')) p += 'index.html';
  const f = path.join(root, p);
  if (!f.startsWith(root) || !fs.existsSync(f)) { res.writeHead(404); res.end('nf'); return; }
  let body = fs.readFileSync(f);
  if (f.endsWith('.html')) body = Buffer.from(body.toString().replace('<head>', `<head><script>${mock}</script>`));
  res.writeHead(200, { 'content-type': types[path.extname(f)] || 'application/octet-stream' });
  res.end(body);
}).listen(port, '127.0.0.1', () => console.log('preview on ' + port));
