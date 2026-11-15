// Fonts are bundled (no network fonts: the CSP is `default-src 'self'`).
import '@fontsource-variable/inter/wght.css';
import '@fontsource/jetbrains-mono/400.css';
import '@fontsource/jetbrains-mono/500.css';
import './lib/design/base.css';

import { mount } from 'svelte';
import App from './App.svelte';

const target = document.getElementById('app');
if (!target) throw new Error('#app element missing');

export default mount(App, { target });
