// A minimal Stream Deck plugin using only Node's built-in WebSocket (Node 22+).
const args = process.argv.slice(2);
const arg = (name) => args[args.indexOf(name) + 1];
const info = JSON.parse(arg("-info"));
const ws = new WebSocket(`ws://127.0.0.1:${arg("-port")}`);
const counts = {};
const send = (m) => ws.send(JSON.stringify(m));
ws.onopen = () => {
  send({ event: arg("-registerEvent"), uuid: arg("-pluginUUID") });
  send({ event: "logMessage", payload: { message: `started, ${info.devices.length} device(s)` } });
  send({ event: "getGlobalSettings", context: arg("-pluginUUID") });
};
ws.onmessage = (e) => {
  const m = JSON.parse(e.data);
  const s = m.payload?.settings ?? {};
  switch (m.event) {
    case "willAppear":
      counts[m.context] = s.count ?? 0;
      send({ event: "setTitle", context: m.context, payload: { title: String(counts[m.context]) } });
      break;
    case "keyDown": {
      const n = (counts[m.context] ?? 0) + (s.step ?? 1);
      counts[m.context] = n;
      send({ event: "setSettings", context: m.context, payload: { ...s, count: n } });
      send({ event: "setTitle", context: m.context, payload: { title: String(n) } });
      send({ event: "setState", context: m.context, payload: { state: n % 2 } });
      const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="72" height="72"><rect width="72" height="72" fill="#00aa00"/></svg>`;
      send({ event: "setImage", context: m.context, payload: { image: "data:image/svg+xml;charset=utf8," + encodeURIComponent(svg) } });
      send({ event: "showOk", context: m.context });
      send({ event: "setGlobalSettings", context: arg("-pluginUUID"), payload: { presses: n } });
      break;
    }
    case "didReceiveGlobalSettings":
      send({ event: "logMessage", payload: { message: `global ${JSON.stringify(m.payload.settings)}` } });
      break;
  }
};
