// Tiny client for the Solus automation channel.
// usage: node tools/auto.mjs <port> '<json cmd>' ['<json cmd>' ...]   (token from SOLUS_AUTOMATION_TOKEN)
import net from 'net';
export function connect(port, token) {
  const sock = net.connect(port, '127.0.0.1');
  let buf = '';
  const waiters = [];
  sock.on('data', (d) => {
    buf += d;
    let i;
    while ((i = buf.indexOf('\n')) >= 0) { const line = buf.slice(0, i); buf = buf.slice(i + 1); const w = waiters.shift(); w && w(JSON.parse(line)); }
  });
  const ready = new Promise((res, rej) => { sock.on('connect', res); sock.on('error', rej); });
  return {
    ready,
    send(cmd, ms = 45000) {
      return new Promise((res, rej) => {
        let done = false;
        const t = setTimeout(() => { done = true; rej(new Error('automation timeout: ' + cmd.cmd)); }, ms);
        // a timed-out reply still arrives later; keep the queue aligned by consuming it
        waiters.push((v) => { clearTimeout(t); if (!done) res(v); });
        sock.write(JSON.stringify({ ...cmd, token }) + '\n');
      });
    },
    close() { sock.end(); },
  };
}
if (process.argv[1] && process.argv[1].endsWith('auto.mjs')) {
  const [, , port, ...cmds] = process.argv;
  if (port) {
    const c = connect(+port, process.env.SOLUS_AUTOMATION_TOKEN);
    await c.ready;
    for (const s of cmds) console.log(JSON.stringify(await c.send(JSON.parse(s))));
    c.close();
  }
}
