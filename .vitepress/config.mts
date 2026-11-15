import { defineConfig } from "vitepress";
import llmstxt from "vitepress-plugin-llms";

export default defineConfig({
  vite: {
    plugins: [llmstxt()],
  },
  title: "sturn",
  description: "evented coordination over the git object store — agents murmurate, not merge",
  base: "/sturn/",
  ignoreDeadLinks: [/\/api\//],
  themeConfig: {
    nav: [
      { text: "Tutorials", link: "/tutorials/getting-started" },
      { text: "How-to", link: "/how-to/hydrate" },
      { text: "Explanation", link: "/explanation/architecture" },
      { text: "Reference", link: "/reference/ops" },
      {
        text: "LLMs",
        items: [
          { text: "llms.txt", link: "https://9aia.com/sturn/llms.txt", target: "_blank" },
          { text: "llms-full.txt", link: "https://9aia.com/sturn/llms-full.txt", target: "_blank" },
        ],
      },
      { text: "9aia", link: "https://9aia.com" },
    ],
    sidebar: [
      {
        text: "Tutorials",
        items: [{ text: "Your first swarm", link: "/tutorials/getting-started" }],
      },
      {
        text: "How-to",
        items: [
          { text: "Hydrate a workspace", link: "/how-to/hydrate" },
          { text: "Mount a workspace", link: "/how-to/mount" },
          { text: "Run the daemon", link: "/how-to/daemon" },
        ],
      },
      {
        text: "Explanation",
        items: [
          { text: "Architecture", link: "/explanation/architecture" },
          { text: "Concepts", link: "/explanation/concepts" },
          { text: "Decisions", link: "/explanation/decisions" },
          { text: "Why no CRDT for code", link: "/explanation/no-crdt" },
          { text: "The model", link: "/explanation/model" },
        ],
      },
      {
        text: "Reference",
        items: [
          { text: "Ops", link: "/reference/ops" },
          { text: "CLI", link: "/reference/cli" },
          { text: "Config", link: "/reference/config" },
          { text: "Crate API", link: "/reference/crate-api" },
        ],
      },
    ],
    socialLinks: [{ icon: "github", link: "https://github.com/9aia/sturn" }],
  },
});
