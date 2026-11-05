import { View, div } from "gpui-kit";
import { v_flex, Button } from "gpui-base";
import { show_fps_monitor, hide_fps_monitor } from "gpui-fps";
import { getContext, getWorkspaceMetadata } from "conduit";

export default class Summary extends View {
  init() { this.count = 0; }
  on_command(name, payload, cx) {
    if (name === "increment") {
      this.count += 1;
      cx.notify();
      return { count: this.count, owner: getContext().pluginId };
    }
    if (name === "read-after-delay") {
      const data = getWorkspaceMetadata();
      return cx.sleep(1000).then(() => ({ workspace: data.id, count: this.count }));
    }
    if (name === "overlays") {
      const kind = payload?.kind ?? "all";
      if (kind === "delayed") {
        return cx.sleep(1000).then(() => this.openOverlays("all"));
      }
      return this.openOverlays(kind);
    }
    throw new Error("unknown summary command");
  }
  openOverlays(kind) {
    const owner = getContext().pluginId;
    if (kind === "clear") {
      const closed = window.close_all_dialogs();
      window.close_sheet();
      window.clear_toasts();
      hide_fps_monitor();
      return { closed };
    }
    if (kind === "all" || kind === "sheet") {
      window.open_sheet(() => v_flex().gap(12)
        .child(div().child("Plugin sheet"))
        .child(div().child(owner)));
    }
    if (kind === "all" || kind === "dialog") {
      window.open_dialog(() => v_flex().gap(12)
        .child(div().child("Plugin dialog"))
        .child(div().child(owner))
        .child(Button.new("plugin-dialog-close").p(8).border_1()
          .on_click(() => window.close_dialog()).child("Close dialog")));
    }
    if (kind === "all" || kind === "extras") {
      window.push_toast({ id: "overlay-status", title: owner, timeout: null });
    }
    if (kind === "extras") show_fps_monitor();
    return { owner };
  }
  render(cx) {
    return v_flex().p(16).gap(12).text_color(cx.theme().colors.foreground)
      .child(div().child("Development summary"))
      .child(div().child(getWorkspaceMetadata()?.name ?? "No workspace"))
      .child(div().child(`Command count: ${this.count}`))
      .child(Button.new("plugin-open-overlays").p(8).border_1()
        .tooltip("Open this plugin's dialog and sheet")
        .on_click(() => this.openOverlays("all")).child("Open plugin overlays"));
  }
}
