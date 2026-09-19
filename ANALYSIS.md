# whatlang-rs 检测流程分析

本文沿公开入口 `detect` / `detect_with_options` 追到脚本统计、候选过滤、profile 距离、
排序与置信度计算。所有行号基于本仓库当前源码，可由文末复现命令逐条核对。

## 1. 调用链总览

- `detect(text)` 用默认 `Options` 转发给 `detect_with_options`（src/core/detect.rs:30、src/core/detect.rs:35）。
- 默认 `Options`：`filter_list = FilterList::All`，`method = Method::Combined`（src/core/options.rs:11-14）。
- `detect_with_options` 构造 `Query` 后进入 `detect_by_query`（src/core/detect.rs:44），这是真正的分发点：
  1. `raw_detect_script(text)` 做脚本统计（src/core/detect.rs:45）；
  2. `main_script()` 取计数最高的脚本，若所有脚本计数为 0（纯标点/数字）直接返回 `None`（src/core/detect.rs:46，src/scripts/detect.rs:34-41）；
  3. `script.to_lang_group()` 把脚本映射成三类语言组并分发（src/core/detect.rs:48-57，映射表在 src/scripts/grouping.rs:37-69）。

## 2. 脚本统计

`raw_detect_script`（src/scripts/detect.rs:52）对 25 个脚本各维护一个计数器，逐字符分类；
`is_stop_char`（src/utils.rs:5）把空格、标点、数字排除在统计之外。命中脚本后与前一计数器
`swap`（src/scripts/detect.rs:105），让主导脚本排在前面加速后续字符。因此混合脚本文本
只看**多数派脚本**，少数派脚本字符不会触发另一条检测路径。

## 3. 三条分发路径：哪类输入跳过 trigram

- `ScriptLangGroup::One(lang)`：希腊文、韩文、泰文、格鲁吉亚文等 20 种脚本一一对应单一语言
  （src/scripts/grouping.rs:48-68），直接返回 `Info::new(script, lang, 1.0)`（src/core/detect.rs:52），
  **不计算 trigram**，置信度硬编码 1.0。
- `ScriptLangGroup::Mandarin`：中文/日文启发式（src/core/detect.rs:76-104），按汉字与假名
  计数比例 `jpn_pct` 分档（>0.2 → Jpn/1.0，>0.05 → Jpn/0.5，>0.02 → Cmn/0.5，否则 Cmn/1.0，
  src/core/detect.rs:91-96），**不计算 trigram**。注意：若 `Cmn` 被过滤掉，无论文本内容如何
  都返回 `(Jpn, 1.0)`（src/core/detect.rs:99-101）。
- `ScriptLangGroup::Multi`：Latin、Cyrillic、Arabic、Devanagari、Hebrew 五种多语言脚本，
  按 `method` 分发到 alphabet / trigram / combined（src/core/detect.rs:62-73）。
  只有 `Method::Alphabet` 会跳过 trigram；默认的 `Method::Combined` **总是**同时跑两个
  子方法（src/combined/mod.rs:37-38）。

## 4. 候选过滤

`FilterList` 是枚举 `All | Allow(Vec<Lang>) | Deny(Vec<Lang>)`（src/core/filter_list.rs:6-11），
判定逻辑集中在 `is_allowed`（src/core/filter_list.rs:26-32）。过滤发生在两个地方：

- trigram：遍历静态 profile 列表时跳过被否语言（src/trigrams/detection.rs:75-78）；
- alphabet：汇总分数前过滤（src/alphabets/common.rs:88-90）。

若某脚本全部语言被过滤，候选为空，`detect` 返回 `None`
（src/core/detect.rs:158 起的既有测试 `test_detect_with_options_with_filter_list_except_none` 覆盖此行为）。

**whitelist 与 blacklist 冲突时谁生效**：不可能冲突。`Options` 只持有一个 `FilterList`
枚举值，`set_filter_list` 整体覆盖（src/core/options.rs:30-33），`Detector::with_allowlist` /
`with_denylist` 也只是构造不同的枚举（src/core/detector.rs:38-46）。后设置者生效，
不存在"同时 allow 又 deny"的状态。

## 5. Profile 距离、阈值与最小长度

trigram 路径（src/trigrams/detection.rs:63-98）：

1. 文本小写化后统计 trigram 频次，按频次降序取前 `TEXT_TRIGRAMS_SIZE = 600` 个，
   映射为"trigram → 名次"（src/trigrams/utils.rs:40，常量 src/trigrams/mod.rs:19）。
2. 每个语言 profile 是 300 个 trigram 的有序列表（src/trigrams/profiles.rs:7-8）。
   距离 = 每个 profile trigram 与文本中名次的绝对差，缺失记 `MAX_TRIGRAM_DISTANCE = 300`
   （src/trigrams/detection.rs:101-119，常量 src/trigrams/mod.rs:13）。
3. **最小长度补偿**：当文本唯一 trigram 数 `count < 300` 时，减去
   `(300 - count) * 300`（src/trigrams/detection.rs:115-117），避免短文本因"装不下
   300 个 trigram"被系统性惩罚；同时 `max_dist = count * 300`（src/trigrams/detection.rs:89）
   让归一化分数随文本长度缩放。
4. 距离转分数：`score = (max_dist - dist) / max_dist`（src/trigrams/detection.rs:123-126）。

Combined 模式下另有长度相关权重：字符数 < 100 时 alphabet 权重从 2/3 线性下降，
≥100 后固定在 1/3（`calc_alphabet_weight`，src/combined/mod.rs:115-118），即**短文本更依赖
alphabet，长文本更依赖 trigram**。置信度侧：`confident_rate = 3/count + 0.015`
（src/core/confidence.rs:21），`count` 越小，前两名需要拉开的相对差距越大才能拿到
高置信度——短文本天然低置信。`is_reliable` 的阈值是 0.9（src/core/info.rs:3）。

## 6. 排序与同分候选的稳定性

三处排序都用 `sort_unstable`：trigram 按距离升序（src/trigrams/detection.rs:87）、
alphabet 按分数降序（src/alphabets/common.rs:93）、combined 按加权分降序
（src/combined/mod.rs:82）。同分候选能**稳定复现**的原因是：

- 排序输入的顺序是确定的：trigram 候选按静态数组 `LATIN_LANGS` 等的声明顺序生成
  （src/trigrams/detection.rs:74-81），alphabet 候选按 `script.langs()` 的静态顺序生成
  （src/alphabets/common.rs:88-95），不经过哈希表迭代；
- Rust 的 `sort_unstable`（pdqsort 族）对同一输入序列是确定性算法，同一二进制对同一
  文本多次运行必然给出相同的同分次序。

注意这是"可复现"而非"稳定排序"：`sort_unstable` 不保证相等元素的相对顺序，同分时
谁排前面是实现细节，跨 Rust 版本升级可能改变（见文末"推测与未确认事项"）。

## 7. 置信度计算

`calculate_confidence(highest, second, count)`（src/core/confidence.rs:6-29）：

- 最高分为 0 → 0.0；第二名为 0 → 直接取最高分；
- 否则 `rate = (highest - second) / second` 与 `confident_rate = 3/count + 0.015` 比较，
  超过则 1.0，否则按比例折算；
- **只剩一个候选时没有第二名，置信度直接硬编码 1.0**（src/combined/mod.rs:26-30，
  trigram/alphabet 单方法同理）。所以"白名单只剩一门语言 → confidence 1.0"不表示
  识别有把握，只表示没有竞争对手。

## 8. 五个风险点（最小文本 + 选项 + 值得观察的中间量）

1. **短文本被 alphabet 主导且置信度趋零**
   - 文本：`"Bonjour"`；选项：默认。
   - 实测：判为 `Uzb`（错误），confidence ≈ 0.0082。
   - 观察：`trigrams_count = 7`（唯一 trigram 数）、`calc_alphabet_weight(7) ≈ 0.643`
     （alphabet 权重接近上限 2/3）、`confident_rate = 3/7 + 0.015 ≈ 0.4436`。
2. **混合脚本只看多数派，少数派字符变成噪声 trigram**
   - 文本：`"This English sentence quietly contains the Russian word любовь inside it."`；选项：默认。
   - 实测：脚本计数 Latin 56 / Cyrillic 6，走 Latin 路径；`любовь` 的字符不是 stop char，
     会进入 trigram 统计成为垃圾 trigram；Eng 0.5263 vs Fra 0.5061，confidence ≈ 0.6911，
     `is_reliable() = false`。
   - 观察：`RawScriptInfo.counters` 前两名、combined 前两名分数、`confidence`。
3. **过滤改变竞争格局，亚军的置信度可能反而满分**
   - 文本：同上；选项：`FilterList::deny(vec![Lang::Eng])`。
   - 实测：Eng 被排除后 Fra 胜出且 confidence = 1.0（rate ≈ 0.063 > 3/70+0.015 ≈ 0.0578）。
     若用 `FilterList::allow(vec![Lang::Eng])` 单候选白名单，任何文本 confidence 恒为 1.0。
   - 观察：`trigram_candidates`（37 → 36 或 1）、前两名分数差、`confidence`。
4. **Mandarin 路径下过滤 Cmn 会无视文本内容**
   - 文本：`"東京は水と木"`（汉字为主、含假名）；选项：`FilterList::deny(vec![Lang::Cmn])`。
   - 实测：直接返回 `(Jpn, 1.0)`，根本不统计假名比例（src/core/detect.rs:99-101）。
   - 观察：`is_allowed(Lang::Cmn)`、`jpn_pct`（未过滤时 2/6 ≈ 0.333 > 0.2 也是 Jpn/1.0，
     两种路径结果撞车，只能靠是否进入启发式分支区分）。
5. **Arabic / Devanagari / Hebrew 的 alphabet 是 mock，排名实际由 trigram 单独决定**
   - 文本：`"האקדמיה ללשון העברית"`；选项：默认。
   - 实测：alphabet 对 Heb、Yid 都给 1.0（src/alphabets/detection.rs:35-46 的 `build_mock`），
     combined 分数 = `1.0 * w_a + t * w_t`，语言间差异完全来自 trigram 项；
     Heb 0.7574 vs Yid 0.7017，confidence ≈ 0.4807。
   - 观察：`alphabet_raw_outcome.scores`（全等）、`trigram_raw_outcome.scores`、
     `calc_alphabet_weight(count)`。

## 9. 测试辅助器（仅测试编译期可见）

新增 `src/core/probe.rs`，仅在 `cfg(test)` 下编译（src/core/mod.rs:8-9），不进发布 API。
它不复制检测算法，只做两件事：

- 在真实分发点记录走了哪条分支（埋点：src/core/detect.rs:49-50）；
- 在真实 trigram 评分处记录过滤后的候选数（埋点：src/trigrams/detection.rs:83-84）。

`observe(text, options)` 调用真实的 `detect_with_options` 并返回
`Observation { path, lang, trigram_computed, trigram_candidates }`。
`src/core/probe.rs` 中的 5 个 `understanding_*` 测试用同一段拉丁-西里尔混合文本
（`"This English sentence quietly contains the Russian word любовь inside it."`）验证：

- 无过滤：Multi 路径、计算 trigram、37 个候选、判 Eng；
- 白名单 `[Eng, Deu]`：同路径、候选缩到 2、仍判 Eng；
- 黑名单 `[Eng]`：同路径、候选 36、改判 Fra；
- 另两个用例证明希腊文（One）和中日混合（Mandarin）路径不计算 trigram。

每个用例都同时断言最终语言、候选数和是否计算 trigram，而非只测一个枚举值。

另有 `tests/understanding.rs`（`harness = false` 的自定义测试目标，见 Cargo.toml），
只通过公开 API 重放上述三种过滤场景，并把各验证阶段名称直接打印到
`cargo test --quiet understanding` 的输出里。

## 10. 复现命令

在仓库根目录直接执行，无需外部服务、环境变量或网络：

```sh
cargo build --all-targets        # 准备阶段
cargo test --quiet understanding # 验收：5 个 understanding_* 用例
```

输出中可见 `understanding stage: ...` 各阶段名与 `understanding: all stages passed`。

说明：`benches/example.rs` 依赖 `dev` feature 的内部 API，本次在 `Cargo.toml` 的
`[[bench]]` 上补了 `required-features = ["dev"]`，否则 `cargo build --all-targets`
（不启用 dev）无法编译该 bench。profile 数据、公开阈值与 crate 的 std/no_std 形态均未改动。

## 11. 推测与未确认事项

- 第 6 节"同分可复现"中，"同一二进制多次运行结果一致"可由源码确定性推出；
  但"跨 Rust 版本同分次序是否变化"取决于 std 排序实现细节，本文未逐一验证各版本，
  不作为既定事实。
- 第 8 节风险点 2 中"垃圾 trigram 拉低了 Eng 与 Fra 的分差"是合理推断：
  源码上只能确认西里尔字符会进入 trigram 统计（`to_trigram_char` 只把标点数字
  归一为空格，src/trigrams/utils.rs:86），具体拉低多少未单独量化。
