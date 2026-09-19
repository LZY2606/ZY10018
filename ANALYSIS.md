# whatlang-rs 检测链路分析

本文沿公开入口 `detect` / `detect_with_options` 追到脚本统计、候选过滤、profile 距离、
排序与置信度计算，并用一组仅测试期可用的观测钩子（`src/observe.rs`）和
`src/understanding_tests.rs` 中的用例逐条验证文中断言。

## 复现命令

在仓库根目录直接执行，无需外部服务、环境变量或网络：

```sh
cargo build --all-targets        # 准备阶段（bench 目标需要 dev feature，见 Cargo.toml）
cargo test --quiet understanding # 验收：运行本文全部验证用例并显示用例名
```

`--quiet` 下 libtest 只画圆点，因此每个用例通过 `std::io::stderr()` 直写
（绕过 libtest 输出捕获）打印自己的名字，验收输出中可见
`case understanding_* ... running` 五行。

## 一、调用链总览

1. `detect(text)` 用默认 `Options`（`filter_list = All`，`method = Combined`）
   转发给 `detect_with_options`（`src/core/detect.rs:30-42`，默认值见
   `src/core/options.rs:11-16`）。
2. `detect_by_query` 先做脚本统计：`raw_detect_script(query.text)`，再取
   `main_script()`；若文本没有任何脚本字符（纯数字/标点，即 stop char，
   定义见 `src/utils.rs:4-7`），`main_script()` 返回 `None`，整个检测直接
   返回 `None`（`src/core/detect.rs:44-46`，`src/scripts/detect.rs:34-39`）。
3. 脚本统计本身是一次单遍计数：25 个 `(Script, check_fn, count)` 计数器
   （`src/scripts/detect.rs:52-79`），命中即前移一位以加速后续比较
   （`src/scripts/detect.rs:88-109`），最后按计数降序排序
   （`src/scripts/detect.rs:29-32`）。**混合脚本文本只取计数最高的主脚本**，
   少数派脚本的字符不会触发另一条检测路径，而是作为噪声进入后续打分。
4. 主脚本经 `Script::to_lang_group()` 分三类（`src/scripts/grouping.rs:37-67`）：
   - `One(lang)`：Hangul→Kor、Greek→Ell、Thai→Tha 等 19 种单语言脚本
     （`src/scripts/grouping.rs:48-65`），直接返回 `Info { confidence: 1.0 }`
     （`src/core/detect.rs:49-53`）。
   - `Mandarin`：中文/日文判别启发式（`src/core/detect.rs:81-109`）。
   - `Multi(...)`：Latin/Cyrillic/Arabic/Devanagari/Hebrew，进入计分
     （`src/core/detect.rs:67-76`），按 `Method` 分发到
     `alphabets::detect` / `trigrams::detect` / `combined::detect`。

## 二、哪类输入会跳过 trigram

以下输入**不会**执行 trigram 统计（由 `understanding_skips_trigram_paths` 验证）：

1. 纯 stop char 文本（如 `"12345 !!!"`）：`main_script()` 为 `None`，
   返回 `None`，任何计分都不发生（`src/core/detect.rs:46`）。
2. 主脚本属 `ScriptLangGroup::One`（如韩文 `"한국어는 …"`）：直接给
   `confidence = 1.0`，不过滤、不打分（`src/core/detect.rs:49-53`）。
3. 主脚本为 Mandarin（如 `"水"`）：走假名比例启发式
   （`src/core/detect.rs:85-108`），不构造 trigram。
4. `Method::Alphabet`：只跑字母表计分（`src/core/detect.rs:73`）。注意
   `set_method` 仅在 `dev` feature 下可用（`src/core/options.rs:33-37`），
   公开 API 用户实际无法选择；默认 `Combined` 总是同时跑 alphabet 与
   trigram（`src/combined/mod.rs:37-38`）。

反向地，Arabic/Devanagari/Hebrew 三个脚本的 alphabet 计分是 mock
（`src/alphabets/detection.rs:35-41` 的 `build_mock`），这些脚本的区分
实际完全依赖 trigram。

## 三、候选过滤：whitelist 与 blacklist 谁生效

- `FilterList` 是枚举：`All | Allow(Vec<Lang>) | Deny(Vec<Lang>)`
  （`src/core/filter_list.rs:5-10`），`Options` 同一时刻只持有一个
  （`src/core/options.rs:5-8`）。因此白名单与黑名单**结构上不可能同时
  生效**；`set_filter_list` 是覆盖式 builder（`src/core/options.rs:28-31`），
  重复调用时**后设置者生效**。
- `Allow` 的语义是"列表外一律拒绝"（`src/core/filter_list.rs:29`），
  即白名单本身已隐含黑名单，无需组合。
- 过滤发生在**打分之前**：trigram 侧在遍历 profile 列表时 `continue` 掉
  被禁语言（`src/trigrams/detection.rs:75-81`），被过滤语言不参与排序，
  也不参与置信度的 top1/top2 比较。候选数因此 = profile 表中属于该脚本
  且通过过滤的语言数（Latin 为 `LATIN_LANGS.len()`，见
  `src/trigrams/profiles.rs:11`）。
- 两个真实的"冲突"反例（由 `understanding_filter_quirks` 验证）：
  - Mandarin 路径只查询 `is_allowed(Lang::Cmn)`（`src/core/detect.rs:85`）。
    当 Cmn 被禁时走 else 分支**无条件**返回 `(Jpn, 1.0)`
    （`src/core/detect.rs:106`）——即使 Jpn 也在黑名单里。对
    `"水"` 设置 `deny([Cmn, Jpn])` 仍返回 `Jpn`。
  - `One(lang)` 路径完全不看过滤表（`src/core/detect.rs:49-53`）：
    对韩文文本设置 `deny([Kor])` 仍返回 `Kor`。

## 四、profile 距离、排序与同分稳定性

1. 文本侧：小写化后统计 trigram 频次，按 `(count, trigram)` 二元组降序
   排序取前 `TEXT_TRIGRAMS_SIZE = 600` 个，名次即位置
   （`src/trigrams/utils.rs:28-44`，常量见 `src/trigrams/mod.rs:13-18`）。
   等频次时按 trigram 字典序决胜，因此文本 profile 的位置编号是唯一确定的。
2. 距离：对每种候选语言，遍历其 300 个 profile trigram，与文本位置求
   绝对差，缺失计 `MAX_TRIGRAM_DISTANCE = 300`
   （`src/trigrams/detection.rs:104-122`）。文本唯一 trigram 数不足 300 时
   做 `delta` 补偿（`src/trigrams/detection.rs:117-120`），结果 clamp 到
   `MAX_TOTAL_DISTANCE = 90_000`。
3. 打分：`score = (max_dist - distance) / max_dist`，其中
   `max_dist = 唯一trigram数 × 300`（`src/trigrams/detection.rs:89-93`，
   `src/trigrams/detection.rs:126-129`）。文本越短分母越小，分数分布越极端。
4. 排序：trigram 侧按距离升序 `sort_unstable_by_key`
   （`src/trigrams/detection.rs:84`）；combined 侧把 alphabet 与 trigram
   分数按 `calc_alphabet_weight` 加权合并后按分数降序
   `sort_unstable_by`（`src/combined/mod.rs:52-89`，权重函数
   `src/combined/mod.rs:115-118`：字符数 0→2/3、100 及以上→1/3）。
5. **同分为何能稳定返回**：候选 profile 列表是编译期静态数组，遍历顺序
   固定（`src/trigrams/profiles.rs:11` 起）；`sort_unstable*` 无随机源，
   同一输入排列必产生同一输出排列，因此同一二进制对同一文本的重复调用
   结果完全一致（`understanding_deterministic_results` 连跑 11 次断言
   `Info` 相等）。注意：`sort_unstable` 不是稳定排序，等值元素的相对顺序
   是实现细节——同一工具链内确定，跨 Rust 版本是否不变无法从本仓库源码
   确认（见"未确认的推测"）。

## 五、置信度：阈值与最小长度如何共同作用

`calculate_confidence`（`src/core/confidence.rs:5-29`）只用三个量：
top1 分数、top2 分数、`count`（trigram/combined 路径为唯一 trigram 数，
见 `src/combined/mod.rs:17` 与 `src/trigrams/detection.rs:37`；alphabet
路径为字符数，`src/alphabets/detection.rs:17`）。

- 置信线 `confident_rate = 3.0 / count + 0.015`
  （`src/core/confidence.rs:21`）：**文本越短，count 越小，要求的相对
  领先幅度越大**。`"hi"` 只有 2 个唯一 trigram，confident_rate 约 1.5，
  几乎不可能满分；长文本阈值趋近 1.5%，很容易到 1.0。
- `rate = (score1 - score2) / score2` 超过 confident_rate 则 confidence
  为 1.0，否则按比例缩放（`src/core/confidence.rs:22-28`）。
- 只剩一个候选时没有 top2，直接给 `confidence = 1.0`
  （`src/combined/mod.rs:26-30`、`src/trigrams/detection.rs:36-40`）。
  这就是"白名单缩窄到 1 个候选时置信度恒 1.0"的来源，是过滤的副产物，
  不代表匹配质量。
- `is_reliable()` 的公开阈值是 `confidence > 0.9`
  （`src/core/info.rs:3`、`src/core/info.rs:34-36`）。它与上面的长度
  阈值叠加效果：短文本即使 top1 大幅领先，confidence 也被压低，
  `is_reliable()` 为 false；长文本微弱领先即可 reliable。

## 六、五个风险点（最小文本 + 选项 + 值得观察的中间量）

观测方式：`src/understanding_tests.rs` 中的 `run()` 先 `observe::reset()`，
再调 `detect_with_options`，最后读 `observe::snapshot()`，可拿到
`path`（走了哪条分支）、`trigram_computed`、`trigram_candidates`
（过滤后候选数）、`trigrams_count`（唯一 trigram 数）。

1. **极短文本的"最高分"几乎无意义**
   - 文本 `"hi"`，选项默认。实测 `lang = Zul`，confidence 约 0.004。
   - 观察：`trigrams_count = 2`、`confident_rate` 约 1.5、top1/top2 分数差。
   - 验证：`understanding_confidence_thresholds`。
2. **混合脚本文本只按主脚本归类**
   - 文本 `"I love you. Je t'aime. Ich liebe dich. 愛してる。"`，选项默认。
     主脚本 Latin，假名只作噪声；实测 `lang = Deu`，confidence 约 0.14。
   - 观察：`RawScriptInfo.counters` 各脚本计数、`path = Multi`、
     `trigram_candidates = 37`（= `LATIN_LANGS.len()`）。
   - 验证：`understanding_mixed_text_paths` 第 1 段。
3. **白名单缩窄会制造虚假的 confidence = 1.0**
   - 同一文本 + `FilterList::allow(vec![Lang::Eng])`：候选数 1，
     无 top2，confidence 恒 1.0 且 `is_reliable() = true`。
   - 观察：`trigram_candidates = 1`、combined 分支是否进入
     `src/combined/mod.rs:29` 的 `1.0` 兜底。
   - 验证：`understanding_mixed_text_paths` 第 2 段。
4. **黑名单在非计分路径上失效或反常**
   - 文本 `"水"` + `deny([Cmn, Jpn])`：仍返回 `Jpn`（confidence 1.0），
     因为 `src/core/detect.rs:106` 的 else 分支不再查过滤表。
   - 韩文文本 + `deny([Kor])`：仍返回 `Kor`，`One` 分支不过滤。
   - 观察：`path = Mandarin / OneLang`、`trigram_computed = false`、
     `is_allowed(Cmn)` 的取值。
   - 验证：`understanding_filter_quirks`。
5. **同分/近同分候选的顺序是"确定"而非"稳定"**
   - 文本 `"hello world"`，选项默认：实测 `lang = Nld`，
     confidence 约 0.009，头部候选分数挤在一起，任何过滤微调都会换人
     （同文本 `deny([Eng, Fra, Deu])` 后胜者从 Deu 变 Ces，候选 34）。
   - 观察：`raw_distances`（`src/trigrams/detection.rs:15`）中相邻等值项、
     重复运行的 `Info` 是否逐位相等。
   - 验证：`understanding_mixed_text_paths` 第 3 段、
     `understanding_deterministic_results`。

## 七、测试观测入口（不进入发布 API）

- `src/observe.rs`：整个模块 `#[cfg(test)]`（`src/lib.rs` 中
  `#[cfg(test)] mod observe;`），release 构建与公开 API 完全不含它。
- 钩子只有两处、各一行，只记录不干预：
  `src/core/detect.rs:50-61`（记录走了 One/Multi/Mandarin 哪条分支）、
  `src/trigrams/detection.rs:86-87`（记录 trigram 阶段运行了、过滤后
  候选数、唯一 trigram 数）。
- 辅助器不复制检测算法：测试直接调用公开入口 `detect_with_options`，
  只读取钩子写下的观测值，再断言最终语言、候选数、是否计算 trigram
  三类事实（见 `src/understanding_tests.rs`）。
- 同一段混合文本 `"I love you. Je t'aime. Ich liebe dich. 愛してる。"`
  在三种配置下路径同为 `Multi` 且都计算了 trigram，区别只在候选数
  （37 → 1 → 34）与胜者（Deu → Eng → Ces），证明过滤只作用于候选集，
  不改变检测路径（`understanding_mixed_text_paths`）。

## 八、未确认的推测（不作为事实）

- `calculate_confidence` 中常数 `3.0` 与 `0.015` 的来源：源码注释仅称
  "based on experiments"（`src/core/confidence.rs:20`），具体实验数据
  与调参过程无法从仓库确认。
- `sort_unstable*` 对等值元素的排列是 std 实现细节：同一 rustc 版本下
  确定（本文第五节即依赖此事实），但跨版本升级后等值候选的相对顺序
  是否保持不变，无法从本仓库源码确认。
- Mandarin 路径在 Cmn 被禁时无条件返回 Jpn（`src/core/detect.rs:106`）
  是有意设计还是疏漏，源码与注释未说明；`One` 分支忽略过滤表同理。
  本文只描述行为，不断言意图。
- profile 数据（`src/trigrams/profiles.rs`）的语料来源与生成流程不在
  本仓库中，无法确认。
