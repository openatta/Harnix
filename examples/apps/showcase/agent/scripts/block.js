// `prompt.block` — a fixed block, asked for once when a session is built.
function onBlock() {
  return {
    name: "showcase.note",
    order: 480,
    content:
      "showcase-block-7d21: the Showcase application is installed. Its tools are named " +
      "app__example_showcase__<tool>; each one asks before it runs.",
  };
}
