/** Saves text as a file through the webview. The object URL outlives the click: WebKit can abort a download whose URL is revoked in the same task. */
export function downloadText(fileName: string, content: string, type = 'text/plain;charset=utf-8') {
  const url = URL.createObjectURL(new Blob([content], { type }));
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = fileName;
  anchor.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
