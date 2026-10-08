/**
 * Saves `content` through a transient anchor. The object URL is revoked after a delay rather than
 * in the same task as the click, so the webview has started reading the blob before it is freed.
 */
export function downloadFile(name: string, content: string, type = 'text/plain;charset=utf-8') {
  const url = URL.createObjectURL(new Blob([content], { type }));
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = name;
  anchor.click();
  window.setTimeout(() => URL.revokeObjectURL(url), 1000);
}
