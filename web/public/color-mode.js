document.documentElement.dataset.mode = window.matchMedia("(prefers-color-scheme: dark)").matches
  ? "dark"
  : "light";
