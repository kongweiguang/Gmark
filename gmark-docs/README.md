<!-- @author kongweiguang -->

# Gmark 渲染示例

这里是 Gmark 的渲染与文件格式样板目录。打开 `markdown/00-rendering-showcase.md` 可以先看一页混合示例，再按主题查看代码、数学公式、Mermaid 和结构化文件。

这些文档参考了 `C:\dev\other\zditor-docs\zditor-docs-main` 中的代码、数学、Mermaid、电影和编辑指南示例，只保留 Gmark 当前支持并希望长期维护的 Markdown 语法。示例中的正文不依赖账号、网络服务或其它编辑器的文档模型。

## 示例导航

- [完整渲染总览](./markdown/00-rendering-showcase.md)：一页覆盖常见块、行内格式、资源和混排。
- [基础 Markdown 与组合排版](./markdown/01-markdown-basics.md)：标题、段落、列表、任务、表格、Callout、脚注和安全 HTML。
- [代码示例](./markdown/02-code.md)：常见语言、配置文件、HTTP、Diff 和行内代码。
- [数学公式](./markdown/03-math.md)：行内公式、块公式、推导、分段函数、矩阵和表格混排。
- [Mermaid 图表](./markdown/04-mermaid.md)：流程图、时序图、状态图、类图、ER 图和其它图表。
- [交互渲染能力](./markdown/05-interactive-rendering.md)：可见文本查找、折叠、宽表、剪贴板和公式结构模板。
- [结构化数据与 SVG](./data/README.md)：JSON、JSONL、CSV、TSV 和 SVG 的本地小型测试文件。

## 目录分类

```text
gmark-docs/
├── markdown/        Markdown 渲染主题页
└── data/
    ├── json/        JSON 与 JSONL
    ├── table/       CSV 与 TSV
    └── svg/         SVG 源码与预览
```

## 语法覆盖

| 类别 | Gmark 示例 |
| :---: | :---: |
| 行内内容 | 粗体、斜体、删除线、下划线、上标、下标、行内代码、链接、行内数学公式 |
| 块内容 | 标题、段落、引用、Callout、列表、任务列表、表格、分隔线、脚注定义 |
| 媒体与资源 | 标准 Markdown 栅格/SVG 图片、Gmark 资源标题标记、本地相对路径 |
| 专用渲染器 | `$$...$$` 数学块、`mermaid` 代码块、语法高亮代码块 |
| 安全 HTML | `<details>`、`<mark>`、`<u>`、`<kbd>` 以及受限的样式属性 |
| 结构化文件 | JSON Graph、JSONL 记录、CSV/TSV 表格、SVG 预览 |

以上已经覆盖能够通过样本文档直接观察的 Gmark 渲染能力和结构化文件视图。大文件模式按要求暂不提供 fixture；工作区恢复、自动保存、更新、文件操作等应用行为需要运行真实应用验证，不适合伪装成静态样本文档。

## 从参考文档迁移时的边界

- 去掉了参考文档顶部的工具专用字段头；标题和正文直接使用普通 Markdown。
- 参考文档中的块式提示改为 GFM Callout（例如 `> [!NOTE]`），这样在 Gmark 中仍然是可读、可编辑的引用块。
- 参考文档中的参数化卡片、标签、批注和修订链接改成普通链接、Gmark 资源链接或明确的正文说明。
- 栅格图片与 SVG 使用标准 Markdown 图片语法；附件示例使用 Gmark 约定的链接标题，不复制其它工具的路径参数协议。

## 新增 Demo 的约定

后续用于观察渲染效果的 Markdown 文件统一放在 `markdown/`，结构化样本按格式放在 `data/`：

1. 文件使用 `.md` 扩展名，首行保留 `<!-- @author kongweiguang -->`。
2. 优先使用标准 Markdown/GFM 和 Gmark 已实现的扩展；不要引入其它编辑器的字段头或专有参数。
3. 示例尽量自包含，引用仓库资源时使用稳定的相对路径。
4. 新增文件后在本页补一个导航链接，并在提交前检查源码和渲染效果。
5. JSON、JSONL、CSV、TSV 等机器格式不写入会改变数据的注释；在对应说明页维护作者和用途。

## 文件大小边界

这里的 fixture 只用于快速观察功能，保持在几 KB 以内；不要把大文件、构建产物或性能压力数据提交到这个目录。
