(() => {
  'use strict';
  const vscode = acquireVsCodeApi();
  const fields = document.getElementById('fields');
  const status = document.getElementById('status');
  window.addEventListener('message', event => {
    const model = event.data;
    if (!model || model.type !== 'model' || !Array.isArray(model.fields) || model.fields.length > 100
      || typeof model.uri !== 'string' || !Number.isSafeInteger(model.version)
      || !Number.isSafeInteger(model.documentIndex)) return;
    fields.replaceChildren();
    status.textContent = `${model.kind} · document ${model.documentIndex + 1}`;
    for (const field of model.fields) {
      if (!Array.isArray(field.path) || !field.path.length || field.path.length > 12
        || typeof field.label !== 'string') continue;
      const wrapper = document.createElement('label');
      wrapper.className = 'field';
      wrapper.textContent = field.label;
      const input = document.createElement('input');
      input.type = field.type === 'boolean' ? 'checkbox' : ['number', 'integer'].includes(field.type) ? 'number' : 'text';
      if (field.type === 'integer') input.step = '1';
      if (field.type === 'boolean') input.checked = field.value === true;
      else input.value = String(field.value ?? '');
      input.addEventListener('change', () => {
        const value = field.type === 'boolean' ? input.checked
          : ['number', 'integer'].includes(field.type) ? input.valueAsNumber : input.value;
        if (typeof value === 'number' && !Number.isFinite(value)) return;
        vscode.postMessage({ uri: model.uri, version: model.version, documentIndex: model.documentIndex,
          path: field.path, operation: 'set', value });
      });
      wrapper.append(input);
      fields.append(wrapper);
    }
  });
  vscode.postMessage({ type: 'ready' });
})();
