import { View, div } from "gpui-kit";
import { v_flex } from "gpui-base";
import { getContext, getWorkspaceMetadata, getSessionSummary } from "conduit";

export default class Navigation extends View {
  render(cx) {
    const context = getContext();
    const workspace = getWorkspaceMetadata();
    const sessions = getSessionSummary();
    return v_flex().p(16).gap(12).text_color(cx.theme().colors.foreground)
      .child(div().child("Development navigation"))
      .child(div().child(workspace?.name ?? "No workspace"))
      .child(div().child(`Sessions: ${sessions?.count ?? 0}`))
      .child(div().child(`Owner: ${context.pluginId}`));
  }
}
