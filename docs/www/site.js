/* Light progressive enhancement — keep motion intentional and quiet. */
(function () {
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
    return;
  }

  document.documentElement.classList.add("js");

  var primary = document.querySelector(".btn-primary");
  if (!primary) return;

  primary.addEventListener("pointerenter", function () {
    primary.style.boxShadow = "0 0 0 3px oklch(0.88 0.12 102 / 0.35)";
  });
  primary.addEventListener("pointerleave", function () {
    primary.style.boxShadow = "";
  });
})();
