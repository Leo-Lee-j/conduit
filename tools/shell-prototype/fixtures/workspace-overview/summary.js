import { View, div } from "gpui-kit";
import { v_flex } from "gpui-base";
import { Button } from "gpui-component";
import { snapshot } from "conduit-probe";

export default class Summary extends View {
  init() { this.clicks = 0; }
  on_command(name, payload, cx) {
    if (name === "increment") {
      if (!Number.isSafeInteger(payload) || Math.abs(payload) > 100) throw new Error("invalid increment");
      this.clicks += payload;
      cx.notify();
      return this.clicks;
    }
    if (name === "read-after-delay") {
      const data = snapshot("example.workspace-overview.summary");
      return cx.sleep(1000).then(() => ({ workspace: data.workspace_id, count: this.clicks }));
    }
    throw new Error("unknown summary command");
  }
  render(cx) {
    const data = snapshot("example.workspace-overview.summary");
    if (data.fail_render) throw new Error("intentional prototype render failure");
    return v_flex().size_full().p(16).gap(12)
      .text_color(cx.theme().colors.foreground)
      .child(div().text_size(18).child("Script summary"))
      .child(div().child(data.workspace_name))
      .child(div().child(`Context ${data.stamp.context_version}, generation ${data.stamp.generation}`))
      .child(div().child(`Sessions: ${data.sessions}`))
      .child(Button.new("shell-summary-counter").label(`Independent counter: ${this.clicks}`)
        .on_click((_event, cx) => { this.clicks += 1; cx.notify(); }));
  }
}
