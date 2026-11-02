// Google's file picker in the browser (PDT-316, PDT-378), as the import dialog runs it: a document provider's picker
// (`ProviderPicker`), served at /api/google/picker.js. `pick({ account, request })` opens Google's picker as the
// account (a connection id) and resolves to the references of the Docs and Sheets picked, none when cancelled.
// `request(path)` is a GET as PDT's web app: the picker's access token is for its UI only.
// Google's picker: https://developers.google.com/drive/picker

let loading = null;

/** Google's picker script, loaded once. */
function loadPicker() {
  loading ??= new Promise((resolve, reject) => {
    const failed = () => {
      loading = null;
      reject(new Error('Google’s file picker could not be loaded. Is this machine offline?'));
    };
    const s = document.createElement('script');
    s.src = 'https://apis.google.com/js/api.js';
    s.async = true;
    s.onerror = failed;
    s.onload = () => {
      if (!window.gapi) return failed();
      window.gapi.load('picker', {
        callback: () => (window.google?.picker ? resolve(window.google.picker) : failed()),
        onerror: failed,
      });
    };
    document.head.appendChild(s);
  });
  return loading;
}

/** Opens Google's picker as `account`: the references (`<connection id>/<file id>`) of the files picked. */
export async function pick({ account, request }) {
  const cfg = await request(`/api/google/accounts/${encodeURIComponent(account)}/picker`);
  const g = await loadPicker();
  return new Promise((resolve) => {
    const view = new g.DocsView(g.ViewId.DOCS)
      .setMimeTypes('application/vnd.google-apps.document,application/vnd.google-apps.spreadsheet')
      .setIncludeFolders(true);
    new g.PickerBuilder()
      .addView(view)
      .enableFeature(g.Feature.MULTISELECT_ENABLED)
      .enableFeature(g.Feature.SUPPORT_DRIVES)
      .setOAuthToken(cfg.accessToken)
      .setDeveloperKey(cfg.apiKey)
      .setAppId(cfg.appId)
      .setTitle('Pick Docs and Sheets for PDT')
      .setCallback((d) => {
        if (d.action === g.Action.PICKED) resolve((d.docs ?? []).map((f) => `${account}/${f.id}`));
        else if (d.action === g.Action.CANCEL) resolve([]);
      })
      .build()
      .setVisible(true);
  });
}
