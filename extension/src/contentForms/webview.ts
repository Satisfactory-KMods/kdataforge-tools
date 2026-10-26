/** Host obtains URIs from webview.asWebviewUri and restricts localResourceRoots to media/. */
export function contentFormHtml(cspSource: string, nonce: string, cssUri: string, scriptUri: string): string {
  const localResource = (value: string): boolean => {
    try {
      if (!/^[-A-Za-z0-9+._~:/?&=%]+$/.test(value)) return false;
      const uri = new URL(value);
      return !uri.username && !uri.password && !uri.hash &&
        (uri.protocol === 'https:' && uri.hostname.endsWith('.vscode-cdn.net')
          || uri.protocol === 'vscode-resource:' || uri.protocol === 'vscode-webview-resource:');
    } catch { return false; }
  };
  if (!/^[-a-zA-Z0-9.*:/]+$/.test(cspSource) || !/^[A-Za-z0-9]{16,128}$/.test(nonce)
    || !localResource(cssUri) || !localResource(scriptUri)) {
    throw Error('Unsafe webview resource');
  }
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${cspSource}; script-src 'nonce-${nonce}'; img-src ${cspSource} data:; connect-src 'none'; form-action 'none'; base-uri 'none'"><link rel="stylesheet" href="${cssUri}"></head><body><main><h1>Content form</h1><p id="status" role="status">Loading…</p><div id="fields"></div></main><script nonce="${nonce}" src="${scriptUri}"></script></body></html>`;
}
