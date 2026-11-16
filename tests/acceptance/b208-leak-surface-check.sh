#!/usr/bin/env bash
# B208 泄漏面静态门禁（纯静态断言：只读文件 + `git show`，绝不启动任何进程/服务）。
#
# ## 门禁目标（B208 事故面）
# 1. `crates/pony-agent-core/src/agent/updater.rs` 的自更新子进程必须自成进程组，且终止路径必须收整组
#    （改前是裸 `Command::new`，无 `process_group(0)`；孙进程逃逸即 PPID=1 孤儿）。
# 2. `tests/acceptance/l2at-tui-models*.sh` 起的后台 server 必须由 `trap` 在 EXIT / INT / TERM
#    三条路径上回收，且清理体必须能**按进程组**回收（`kill 0` 或 `kill -TERM -- -"$pid"` 形态）。
# 3. 反误伤正控：`process.rs` 的 B009/B205 进程组收口不得被回退。
# 4. 判别力自证（负控）：用 `git show <改前基线 sha>:<path>` 取**改前**版本，断言改前必然**失败**
#    本条门禁 —— 否则门禁是「永远绿」的假门禁。锚点**不得**用 `HEAD`（提交修复后 HEAD 即修好版本，
#    负控会自破，见 [`B208_PREFIX_BASELINE`]）。
#
# ## 判定口径说明（为什么 `kill 0` 只在非注释行计数）
# 两个脚本第 53 行左右有一句注释 `# 绝不使用 \`kill 0\`：…会连带 TERM 掉调用者。`
# 若朴素 grep 字面量 `kill 0`，该**注释**会让 5 号检查假通过。故本门禁先剥离整行注释再匹配，
# 使通过只能来自**真实可执行语句**（本仓库实际形态是 `kill -TERM -- "-$pid"` 精确组 kill）。
#
# ## 用法
#   bash tests/acceptance/b208-leak-surface-check.sh      # 全过 exit 0；任一 FAIL exit 1
# 退出码：0 = 全部静态断言通过；1 = 至少一条失败。

set -u

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
UPDATER="crates/pony-agent-core/src/agent/updater.rs"
PROCESS="crates/pony-agent-core/src/agent/process.rs"
# B207：范围终结面第二个文件（`RangeOwner` 适配层所在），E9 的跨文件唯一性判据需要它。
PROCESS_RANGE="crates/pony-agent-core/src/agent/process_range.rs"
SCRIPT_A="tests/acceptance/l2at-tui-models.sh"
SCRIPT_B="tests/acceptance/l2at-tui-models-auth.sh"

# B208 负控基线：D8 需要「修复前」的源码。不能用 HEAD（提交修复后 HEAD 就是修好的版本，
# 负控会自破）。此处固定为修复前的最后一个提交；深度克隆下永久可达，浅克隆下走 SKIP。
B208_PREFIX_BASELINE="d21e403aab2e5b149742200e94b3f49cc9ee925b"

FAILED=0
CHECKS=0

pass() {
    CHECKS=$((CHECKS + 1))
    printf '[PASS] %s\n' "$*"
}

fail() {
    CHECKS=$((CHECKS + 1))
    FAILED=$((FAILED + 1))
    printf '[FAIL] %s\n' "$*"
}

# grep -E 断言：文件必须已存在且命中至少一行。
require_match() {
    local desc="$1" file="$2" pattern="$3"
    if [[ ! -f "$REPO_ROOT/$file" ]]; then
        fail "$desc —— 文件不存在：$file"
        return
    fi
    if grep -Eq -- "$pattern" "$REPO_ROOT/$file"; then
        pass "$desc"
    else
        fail "$desc —— 未命中正则：$pattern（文件：$file）"
    fi
}

# 同一文件上「多个候选正则命中任一即 PASS」（命中项回显，作为证据）。
require_match_any() {
    local desc="$1" file="$2"
    shift 2
    if [[ ! -f "$REPO_ROOT/$file" ]]; then
        fail "$desc —— 文件不存在：$file"
        return
    fi
    local pattern
    for pattern in "$@"; do
        if grep -Eq -- "$pattern" "$REPO_ROOT/$file"; then
            pass "$desc（命中：$pattern）"
            return
        fi
    done
    fail "$desc —— 以下正则均未命中：$* （文件：$file）"
}

# 负控断言：给定**文本流**必须不命中任一正则。
require_no_match_stream() {
    local desc="$1" text="$2"
    shift 2
    local pattern
    for pattern in "$@"; do
        if printf '%s' "$text" | grep -Eq -- "$pattern"; then
            fail "$desc —— 改前版本意外命中：$pattern"
            return
        fi
    done
    pass "$desc"
}

# 剥离整行注释（`#` 前只有空白），避免注释文本造成假通过。
strip_comment_lines() {
    sed -E 's/^[[:space:]]*#.*$//' "$1"
}

# 统计**代码行**命中数（`grep -n` 输出形如 `<行号>:<内容>`，故可据前缀剔除注释/属性行）。
# 注释判定与 Rust/Shell 双侧对齐：`//`（含 `///`、`//!`）、`#`（含 `#[attr]`、shell 注释）、
# `*`（块注释续行，如 `* SAFETY: ...`）。
count_code_lines() {
    local file="$1" pattern="$2"
    grep -n -- "$pattern" "$REPO_ROOT/$file" 2>/dev/null \
        | grep -vcE '^[0-9]+:[[:space:]]*(//|#|\*)' || true
}

# ── 判定用正则（集中定义，便于审计）──────────────────────────────────────────────────────────
RE_PROCESS_GROUP='(process_group[[:space:]]*\([[:space:]]*0[[:space:]]*\)|setpgid)'
RE_KILLPG='killpg'
# `trap <cmd> <SIGNAL...>`：信号必须作为独立 token 出现在 trap 语句尾部。
RE_TRAP_EXIT='trap[[:space:]][^;]*[[:space:]](EXIT|0)([[:space:]]|$|;)'
RE_TRAP_INT_TERM='trap[[:space:]][^;]*[[:space:]](INT|TERM)([[:space:]]|$|;)'
# 整组回收原语 ①：`kill 0`（向本进程组全成员发信号）。
RE_KILL_ZERO='(^|[^[:alnum:]_])kill[[:space:]]+0($|[^[:alnum:]_])'
# 整组回收原语 ②：精确组 kill，覆盖 `kill -TERM -- -"$pid"` / `kill -9 -- -$pid` / `kill -- -$pid`。
RE_KILL_GROUP='kill[[:space:]]+(-{1,2}[A-Za-z0-9]+[[:space:]]+)*--?[[:space:]]*-'

printf '== B208 泄漏面静态门禁（repo: %s）==\n' "$REPO_ROOT"

# B5 的具体判定：在剥离注释后的正文里，命中 `kill 0` 或精确组 kill 任一即 PASS。
check_group_reclaim() {
    local stripped="$1" script="$2"
    local hit=""
    if printf '%s\n' "$stripped" | grep -Eq -- "$RE_KILL_ZERO"; then
        hit="kill 0"
    elif printf '%s\n' "$stripped" | grep -Eq -- "$RE_KILL_GROUP"; then
        hit="精确组 kill"
    fi
    if [[ -n "$hit" ]]; then
        pass "B5 $(basename "$script") 清理体含整组回收原语（命中：$hit；注释行已剥离）"
    else
        fail "B5 $(basename "$script") 清理体缺少整组回收原语（既无 kill 0，也无 kill <signal> -- -<pgid> 形态）"
    fi
}

# ── A. updater.rs ────────────────────────────────────────────────────────────────────────────
require_match "A1 updater.rs 存在 process_group(0) 或 setpgid（子进程自成进程组）" \
    "$UPDATER" "$RE_PROCESS_GROUP"
require_match "A2 updater.rs 终止路径出现 killpg（整组回收）" \
    "$UPDATER" "$RE_KILLPG"

# ── B. 两个后台 server 脚本（各 3 条）────────────────────────────────────────────────────────
for script in "$SCRIPT_A" "$SCRIPT_B"; do
    require_match "B3 $(basename "$script") trap 注册覆盖 EXIT/0（正常退出路径回收）" \
        "$script" "$RE_TRAP_EXIT"
    require_match "B4 $(basename "$script") trap 注册覆盖 INT 或 TERM（中断路径回收）" \
        "$script" "$RE_TRAP_INT_TERM"
    # 清理体匹配前剥离整行注释：`kill 0` 只允许来自真实语句。
    if [[ -f "$REPO_ROOT/$script" ]]; then
        check_group_reclaim "$(strip_comment_lines "$REPO_ROOT/$script")" "$script"
    else
        fail "B5 $(basename "$script") 整组回收原语 —— 文件不存在：$script"
    fi
done


# ── C. 反误伤正控：B009/B205 的进程组收口不得被回退 ─────────────────────────────────────────
require_match "C6 process.rs 仍存在 process_group(0)（B009 收口未被回退）" \
    "$PROCESS" "$RE_PROCESS_GROUP"
require_match "C7 process.rs 仍存在 killpg（B009/B205 收口未被回退）" \
    "$PROCESS" "$RE_KILLPG"

# ── E. B206 机器影子（C1-U 唯一终止路径护栏 + 加法式 API 接线）────────────────────────────────
# E9（跨文件属性）：`libc::killpg` 在**范围终结面**（`process.rs` + `process_range.rs`）的
# **代码行合计**恰好 1 处 —— 唯一信号派发点。
#
# 为什么必须两文件合计：B207 之后「唯一派发点」是**跨文件不变量**（`PidRangeOwner::signal`
# → `super::signal_range` → 唯一的 `libc::killpg`）。只查单个文件等于门禁留了盲区：在另一个
# 文件里复制一份 `killpg` 不会被发现，而这正是「第二条信号派发路径」的典型形态。
#
# 文件缺失的处置（fail-closed，不静默通过）：`process_range.rs` 缺失说明 B207 尚未落地，
# 此时**无法**证明跨文件唯一性 —— 门禁给 `[FAIL]` 并显式点名「应报 IMPL-DEFECT」，
# 绝不因为「扫到 0 处」就把它算成 1 处。
killpg_code_count_total=0
killpg_missing_files=""
for killpg_file in "$PROCESS" "$PROCESS_RANGE"; do
    if [[ -f "$REPO_ROOT/$killpg_file" ]]; then
        per_file_count="$(count_code_lines "$killpg_file" 'libc::killpg')"
        per_file_count="${per_file_count:-0}"
        killpg_code_count_total=$((killpg_code_count_total + per_file_count))
    else
        killpg_missing_files="$killpg_missing_files $killpg_file"
    fi
done
if [[ -n "$killpg_missing_files" ]]; then
    fail "E9 无法证明跨文件唯一派发点：缺失${killpg_missing_files}（B207 未落地 ⇒ 按 IMPL-DEFECT 上报，门禁不静默放行）"
elif [[ "$killpg_code_count_total" == "1" ]]; then
    pass "E9 process.rs + process_range.rs 代码行 \`libc::killpg\` 合计恰好 1 处（跨文件唯一派发点，C1-U 护栏）"
else
    fail "E9 process.rs + process_range.rs 代码行 \`libc::killpg\` 必须合计恰好 1 处（跨文件唯一派发点），实际 ${killpg_code_count_total} 处 —— 出现第二处即并行终止路径（C1-U 破坏）"
fi

# E10：加法式 API 在位，且旧的「发了信号就当成功」日志前缀必须消失。
if grep -q 'fn shutdown_checked' "$REPO_ROOT/$PROCESS" 2>/dev/null; then
    legacy_prefix_count="$(grep -c '\[process\] shutdown termination error' "$REPO_ROOT/$PROCESS" 2>/dev/null || true)"
    legacy_prefix_count="${legacy_prefix_count:-0}"
    if [[ "$legacy_prefix_count" == "0" ]]; then
        pass "E10 process.rs 存在 shutdown_checked 且旧前缀 \`[process] shutdown termination error\` 零命中"
    else
        fail "E10 旧前缀 \`[process] shutdown termination error\` 仍有 ${legacy_prefix_count} 处命中（成功口径未改为 fail-loud）"
    fi
else
    fail "E10 process.rs 缺少 fn shutdown_checked（S-3 加法式 API 未接线）"
fi

# ── D. 判别力自证：改前版本必须**失败**本门禁 ───────────────────────────────────────────────
# 锚点（自证型门禁的经典坑）：D8 必须取「修复前」的源码做负控。若锚在 `HEAD` 上，一旦把修复
# 提交进 HEAD，`git show HEAD:` 取到的就是修好的版本 ⇒ 负控自己判红（门禁在提交后自破）。
# 因此锚定到**固定的改前基线 sha**（[`B208_PREFIX_BASELINE`]）；历史被改写 / 浅克隆取不到时
# 优雅降级为 SKIP —— 环境不具备不等于门禁通过，但也不该判红，故仍 exit 0，且必须打印（不许静默）。
if ! git -C "$REPO_ROOT" rev-parse --git-dir >/dev/null 2>&1; then
    fail "D8 负控无法执行：不是 git 仓库或 git 不可用（$REPO_ROOT）"
elif git -C "$REPO_ROOT" cat-file -e "${B208_PREFIX_BASELINE}^{commit}" 2>/dev/null; then
    head_updater="$(git -C "$REPO_ROOT" show "$B208_PREFIX_BASELINE:$UPDATER" 2>/dev/null)" || head_updater=""
    if [[ -z "$head_updater" ]]; then
        fail "D8a 无法用 git show 取得改前的 $UPDATER（基线 $B208_PREFIX_BASELINE）"
    else
        require_no_match_stream \
            "D8a 负控：改前的 updater.rs 不含 process_group/setpgid（门禁在改前必然失败）" \
            "$head_updater" "$RE_PROCESS_GROUP"
    fi
    for script in "$SCRIPT_A" "$SCRIPT_B"; do
        head_script="$(git -C "$REPO_ROOT" show "$B208_PREFIX_BASELINE:$script" 2>/dev/null)" || head_script=""
        if [[ -z "$head_script" ]]; then
            fail "D8b 无法用 git show 取得改前的 $script（基线 $B208_PREFIX_BASELINE）"
            continue
        fi
        head_stripped="$(printf '%s\n' "$head_script" | sed -E 's/^[[:space:]]*#.*$//')"
        require_no_match_stream \
            "D8b 负控：改前的 $(basename "$script") 既无 kill 0 也无精确组 kill（门禁在改前必然失败）" \
            "$head_stripped" "$RE_KILL_ZERO" "$RE_KILL_GROUP"
    done
else
    pass "D8 负控 SKIP：基线提交不可达（浅克隆或历史改写）—— 判别力自证仅在完整克隆下执行"
fi

printf '== 结果：%d 项检查，%d 项失败 ==\n' "$CHECKS" "$FAILED"
if [[ "$FAILED" -ne 0 ]]; then
    exit 1
fi
exit 0
