// The demo deck on the home page: a tiny imitation of the real app's demo deck, with a
// live clock, a press counter, and a few keys that react. A file of its own rather than
// inline code, so the site's Content-Security-Policy can stay at script-src 'self'.
(() => {
  const deck = document.getElementById("deck");
  const caption = document.getElementById("deck-caption");
  if (!deck || !caption) return;
  const keys = [
    ["clock", "--:--", 0], ["counter", "Pressed\n0×", 1], ["Mic\nmuted", "Mic muted", 2, "Mic\nlive"],
    ["Scene:\nGame", "Scene switched", 3, "Scene:\nCam"], ["Lights", "Lights toggled", 4, "Lights\noff"],
    ["Play /\npause", "Music toggled", 5], ["Screen-\nshot", "Screenshot taken", 6], ["Open\nterminal", "Terminal opened", 7],
    ["Go live", "You're live", 1, "End\nstream"], ["Clip it", "Clip saved", 2], ["Volume\nup", "Volume up", 3],
    ["Volume\ndown", "Volume down", 4], ["BRB", "Be right back", 5], ["Copy", "Copied", 6], ["Paste", "Pasted", 7],
    ["Coffee", "Coffee break", 0], ["Lock\nscreen", "Screen locked", 1], ["Timer\n5:00", "Timer started", 2],
    ["Sound-\nboard", "Woof.", 3], ["Fetch", "Good dog.", 4],
  ];
  let presses = 0;
  const els = keys.map(([label, message, tone, alt]) => {
    const b = document.createElement("button");
    b.className = "key";
    b.type = "button";
    b.dataset.tone = String(tone);
    b.textContent = label;
    let toggled = false;
    b.addEventListener("click", () => {
      if (label === "counter") {
        presses += 1;
        b.textContent = `Pressed\n${presses}×`;
        caption.textContent = `Counted ${presses} ${presses === 1 ? "press" : "presses"}.`;
        return;
      }
      if (label === "clock") { caption.textContent = "That clock is live, pushed from the host every second."; return; }
      if (alt) {
        toggled = !toggled;
        b.textContent = toggled ? alt : label;
        caption.textContent = b.textContent.replace(/-?\n/g, (m) => (m.startsWith("-") ? "" : " "));
        return;
      }
      caption.textContent = message;
    });
    deck.appendChild(b);
    return b;
  });
  const clock = els[0];
  const tick = () => {
    clock.textContent = new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  };
  tick();
  setInterval(tick, 1000);
  els[1].textContent = "Pressed\n0×";
})();
