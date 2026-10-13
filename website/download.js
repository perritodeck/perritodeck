// Download page: highlight the visitor's own system and move its card first. Without
// JavaScript all three cards look the same. A file of its own rather than inline code, so
// the site's Content-Security-Policy can stay at script-src 'self'.
(() => {
  const ua = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || navigator.userAgent || "";
  const os = /win/i.test(ua) ? "windows" : /mac/i.test(ua) && !/iphone|ipad/i.test(navigator.userAgent) ? "mac" : /linux|x11/i.test(ua) && !/android/i.test(navigator.userAgent) ? "linux" : "";
  const card = os && document.querySelector(`.os-card[data-os="${os}"]`);
  if (card) {
    card.classList.add("is-yours");
    card.parentNode.prepend(card);
  }
})();
