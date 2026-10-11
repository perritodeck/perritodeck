# PerritoDeck plugin SDK for Node.js

No dependencies; works from Node 18. Copy `index.js` (and `index.d.ts` for
TypeScript) into your plugin's folder as `perritodeck-plugin.js`, or install this
folder as a package.

```js
const { Plugin } = require("./perritodeck-plugin");

const plugin = new Plugin();
plugin.action("say-hello", async (settings) => {
  plugin.log(`hello, ${settings.name ?? "world"}`);
  // return "some text" to flash a warning on the key; throw to flash an error
});
plugin.start();
```

With `"runtime": "node"` and `"main": "plugin.js"` in `perritodeck-plugin.json`
(see [PROTOCOL.md](../PROTOCOL.md)). Run the tests with `node --test`.
