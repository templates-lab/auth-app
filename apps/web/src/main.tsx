/* @refresh reload */
import { render } from "solid-js/web";
import { App } from "./App";
import "./shell/admin.css";

const container = document.getElementById("root");
if (!container) {
  throw new Error("Root element #root not found");
}

render(() => <App />, container);
