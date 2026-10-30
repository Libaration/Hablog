import "@fontsource/silkscreen/400.css";
import "@fontsource/silkscreen/700.css";
import "./style.css";
import { App } from "./app";
import { connect } from "./backend";

void connect().then((backend) => new App(document.getElementById("app")!, backend).init());
