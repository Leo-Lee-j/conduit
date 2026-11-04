import { View, div } from "gpui-kit";
import { v_flex } from "gpui-base";
import * as fs from "fs/promises";

export default class CapabilityProbe extends View {
  init(_props, cx) {
    this.file = "pending";
    this.storage = "pending";
    try {
      localStorage.setItem("probe", "must not be persisted");
      this.storage = "unexpected storage grant";
    } catch (_) { this.storage = "storage denied"; }
    cx.spawn(async (cx) => {
      try {
        await fs.readFile("navigation.js", "utf8");
        this.file = "unexpected filesystem grant";
      } catch (_) { this.file = "filesystem denied"; }
      cx.notify();
    });
  }
  render() { return v_flex().child(div().child(this.file)).child(div().child(this.storage)); }
}
