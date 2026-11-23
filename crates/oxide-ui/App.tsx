/** The workbench: TSX builds the interface, Rust draws the 3D pane. */
import { cleanup, mount } from "nsh";
import { bootstrap } from "./store";
import { Shell } from "./components/Shell";

function App() {
  const stopPolling = bootstrap();
  cleanup(stopPolling);
  return <Shell />;
}

mount(App);
