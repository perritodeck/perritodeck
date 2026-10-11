import { mount } from "svelte";
import "@fontsource-variable/bricolage-grotesque";
import "@fontsource/atkinson-hyperlegible/400.css";
import "@fontsource/atkinson-hyperlegible/700.css";
import "./app.css";
import "./lib/form.css";
import App from "./App.svelte";

export default mount(App, { target: document.getElementById("app")! });
