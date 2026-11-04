import { View, div } from "gpui-kit";
import { v_flex } from "gpui-base";
import { Button } from "gpui-component";
import { snapshot, request_summary, publish_status } from "conduit-probe";

export default class Navigation extends View {
  init() { this.clicks = 0; this.feedback = ""; }
  render(cx) {
    const data = snapshot("example.workspace-overview.index");
    return v_flex().size_full().p(16).gap(12)
      .text_color(cx.theme().colors.foreground)
      .child(div().text_size(18).child("Script navigation"))
      .child(div().child(data.workspace_name))
      .child(div().child(`${data.sessions} synthetic sessions`))
      .child(Button.new("shell-open-summary").label(`Open summary (${this.clicks})`)
        .on_click((_event, cx) => {
          this.clicks += 1;
          request_summary(data.stamp.generation, data.stamp.context_version);
          cx.notify();
        }))
      .child(Button.new("shell-delayed-status").label("Publish status after 1 second")
        .on_click((_event, cx) => {
          cx.spawn(async (cx) => {
            await cx.sleep(1000);
            try {
              publish_status(data.stamp.generation, data.stamp.context_version,
                `Updated: ${data.workspace_name}`);
              this.feedback = "Accepted";
            } catch (_) { this.feedback = "Stale result rejected"; }
            cx.notify();
          });
        }))
      .child(div().child(this.feedback));
  }
}
