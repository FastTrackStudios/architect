import { architect } from '../../webview.js';

await architect.log('about to leave');
location.href = '/test/webview-nav/outside.html';
