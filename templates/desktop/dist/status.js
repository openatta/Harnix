// The shell says what happened by the address it navigates to —
// `index.html?state=error&detail=…` — because the page has no other channel
// to the shell, by design: no IPC, no global Tauri object. Without this file
// (or with script blocked) the page still reads as "starting".
"use strict";

const params = new URLSearchParams(location.search);
if (params.get("state") === "error") {
  document.title = "无法连接";
  document.getElementById("title").textContent = "无法连接";
  // textContent, never innerHTML: the detail is the shell's text, which may
  // quote a path, a URL or a process's exit status.
  document.getElementById("detail").textContent =
    params.get("detail") || "服务没有响应。";
  document.body.classList.add("error");
}
