/**
 * 输入框的草稿：按应用给的 draftKey（通常是会话或锚点）各自保留未发送的内容。
 * 只在这个页面的内存里：输入框卸载、换到别的会话再回来，内容还在；刷新页面就没了。发出去（或清空）即删除。
 */

const drafts = new Map<string, string>();

export function readDraft(key: string | undefined): string {
  return key ? (drafts.get(key) ?? "") : "";
}

export function writeDraft(key: string | undefined, text: string): void {
  if (!key) return;
  if (text) drafts.set(key, text);
  else drafts.delete(key);
}
