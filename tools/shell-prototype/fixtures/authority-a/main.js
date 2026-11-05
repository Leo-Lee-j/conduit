import { View, div } from "gpui-kit";
import { Button } from "gpui-component";
import { identity, report } from "authority";

function inspect(stage) {
  let storage = false;
  try { localStorage.getItem("probe"); storage = true; } catch (_) {}
  report(stage, storage, process.dataDir);
}
inspect("module");
export default class Authority extends View {
  init(_props, cx) {
    inspect("init");
    cx.spawn(async (cx) => {
      await cx.sleep(100);
      inspect("await");
      try { const { secret } = await import("a-only"); report(secret(), false, ""); }
      catch (_) { report("a-only-denied", false, ""); }
      await cx.sleep(100);
      inspect("late");
    }, { owner: null });
  }
  render() {
    return div().child(Button.new(identity()).label(identity())
      .on_click(() => inspect("click")));
  }
}
