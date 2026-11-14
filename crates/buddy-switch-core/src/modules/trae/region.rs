//! Trae 的两个**正交维度**：区域（region）与程序（program）。
//!
//! ## 为什么需要这一层（现状缺陷）
//!
//! 改造前只有一根轴（[`super::variant::TraeVariant`]），两个取值
//! `TraeWork` / `Trae` **都是国内构建**，于是：
//!
//! - `data_dir_names` 把国内与国外的候选名混在同一张表里
//!   （`["TRAE SOLO CN", "TRAE SOLO"]`、`["Trae CN", "Trae"]`），并按顺序取
//!   **第一个存在的**目录 ⇒ 装了国际版也永远探测不到，用户在界面上看不见、管不了；
//! - 「一条程序」与「一个账号体系」被混为一谈：国内的两条程序其实共用
//!   `api.trae.cn` 同一套账号，却各自维护一份账号库。
//!
//! ## 两根轴各自负责什么
//!
//! | 轴 | 取值 | 决定 |
//! |:---|:---|:---|
//! | [`TraeRegion`] | `Cn` / `Global` | **持久化**：账号库 / 登录态快照 / 设备标识 / 冷却 / 端点。即「账号属于哪套账号体系」 |
//! | [`TraeProgram`] | `TraeWork` / `TraeCode` | **执行**：把登录态写进哪个客户端目录、启动哪个 exe |
//!
//! 关键是**持久化轴是区域而不是程序**：CN 与国际是两套互不相通的账号体系
//! （JWT 不通用，端点不同），而同一区域内的两条程序共用同一套账号 ——
//! 同一个 Trae 账号可以分别启用在国内的 TraeWork 与 TraeCode 上。
//!
//! ## ★★ 程序位（program instance）：**每区域恰好一个**（issue #28 收敛）
//!
//! 2026-10-13 客户端把 TraeWork 与 TraeCode 合并成**一个**客户端（CN `Trae CN`、
//! 国际 `Trae`）；2026-10-15（issue #28）本模块跟着**收敛**：
//!
//! | 区域 | 唯一程序位 | userData 目录 | exe / 进程 | 展示名 |
//! |:---|:---|:---|:---|:---|
//! | CN | [`TraeProgram::Trae`] | `Trae CN` | `Trae CN.exe` | `Trae CN` |
//! | Global | [`TraeProgram::Trae`] | `Trae` | `Trae.exe` | `Trae` |
//!
//! **退役的遗留程序位**：`TraeWork`（客户端 `TRAE SOLO*`）与 `TraeCode`（旧 IDE）。
//! 两者**不再是可选程序位**：不在 [`programs_of`] / [`all_program_specs`] 里、
//! 不进界面、不参与探测与启动。
//!
//! 它们**仍然存在于 [`TraeProgram`] 枚举里**，只服务两件事：
//!
//! 1. **解码老数据**：[`TraeProgram::parse`] 仍认 `trae_work` / `trae_code` /
//!    `TRAE SOLO CN` 这些历史写法（老参数、老日志、老配置里都有）；
//! 2. **标识迁移来源**：[`super::snapshot_migrate`] 按遗留程序位枚举**旧快照池目录**，
//!    把它们整组搬进合并版池。
//!
//! ⚠️ 因此 [`program_spec`] 对遗留程序位**返回本区域的合并版 spec**（不是 `None`、
//! 也不是另一张表）—— 「该区域只有一个程序位」这条事实只有一个落点。
//!
//! ⚠️ **候选名仍然必须精确匹配**：`Trae CN` 与 `Trae` 是包含关系，
//! 用「包含匹配」判区域会互相抢（出现「装了 A、读了 B 的目录」），
//! 因此 [`ProgramSpec`] 的候选名一律**逐字比对**。
//!
//! ## 展示名：**国内版带 ` CN` 后缀、国际版不带后缀**
//!
//! 两个名字都取自客户端自己的字符串（`product.json` 的 `nameShort`）：
//! CN 合并客户端 `nameShort` = `Trae CN`、国际版 = `Trae`。
//! ★ 它们与 [`TraeRegion::display_name`] **逐字相同** —— 客户端的产品名本就同时指
//! 「区域」与「那条程序」，用户明确选择如此（见该函数的说明）；两者分属两根轴，
//! **别去重**，也别并进「程序位名两两不同」那类护栏（现在只有一个程序位）。
//!
//! ⚠️ 更正记录（2026-10-13，用户更正）：此前的约定是「国际版带 `AI` 后缀」，那是**错的**
//! —— 它凭空造了 `Trae AI` / `TraeWork AI` 这类客户端根本不用的名字。
//!
//! ## 区域名：**客户端自己的产品名**（用户 2026-10-13 定）
//!
//! [`TraeRegion::display_name`] 返回 **`Trae CN`**（国内）/ **`Trae`**（国际）——
//! 逐字取客户端在该区域的 `nameShort`（本机 `product.json` 实测）。
//!
//! ⚠️ **它与合并版程序位的展示名逐字相同**（`Trae CN` / `Trae`）—— 客户端的产品名
//! 本就同时指这两件事，用户明确选择如此。它们分属**区域轴**与**程序位轴**：
//! ★ 别把它们并进「六个展示名两两不同」那条护栏，★ 也别去重。
//!
//! ⚠️ 它的**已知成本**：文本层面再也无法用 `Trae CN` 当「程序位名」的标记 ——
//! `oauth` 的「成功页不得出现程序位名」断言因此收窄到 `TraeWork CN` / `TraeCode CN`
//! （见该用例的说明）。要恢复强度只能让区域名与程序位名不同。
//!
//! ⚠️ **组合**标签（作用域芯片 = 平台 + 区域词，见前端 `scope-chip.tsx`）用的是
//! **跨平台**的区域**词**（`shared.region.version.*`，WorkBuddy 也读它）⇒
//! 本值塞进那个组合会得到「Trae Trae CN」。Trae 侧因此另有独立词条
//! （前端 `trae.region.*`），本函数只服务 Trae 自己的文案。
//!
//! ## 与 [`super::variant`] 的关系（过渡期）
//!
//! 区域端点**不在这里另立一份**：本模块的 [`region_endpoints`] 直接读
//! [`super::variant::variant_spec`] 里那张已实测/已更正的端点表，
//! 避免同一事实出现两个来源。等调用点全部迁到区域轴后，端点表本身也会搬进来。

use super::variant::{variant_spec, EndpointSet, TraeVariant};

/// 账号体系所在区域。
///
/// 它是**持久化轴**：账号库文件后缀、登录态快照目录、设备标识、冷却账本、
/// 以及全部联网端点都由它决定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TraeRegion {
    /// 国内版（`api.trae.cn` / `api.trae.com.cn`，账号体系与国内主体一致）。
    Cn,
    /// 国际版（客户端 `packageType = SOLO_I18N`，发布者 SPRING (SG) PTE. LTD）。
    Global,
}

impl TraeRegion {
    /// 稳定标识（文件名后缀、序列化、前端传参）。
    pub fn as_str(self) -> &'static str {
        match self {
            TraeRegion::Cn => "cn",
            TraeRegion::Global => "global",
        }
    }

    /// Trae 侧的**区域名**（用户 2026-10-13 定：用客户端自己的产品名）。
    ///
    /// 取客户端在该区域的 `nameShort`（本机 `product.json` 实测）：
    /// 国内版客户端叫 **`Trae CN`**、国际版叫 **`Trae`**（见 [`super::variant`] 模块头）。
    ///
    /// ## ⚠️ 已知的**刻意重名**（用户选择，不是事故）
    ///
    /// 这两个名字与**合并版程序位的展示名**逐字相同（`Trae CN` / `Trae`）——
    /// 因为客户端的产品名本来就同时指这两件事。它们分属两根轴
    /// （**区域** vs **程序位**），因此：
    ///
    /// - ★ **不要**把它们并进「六个展示名两两不同」那条护栏 —— 那条只管程序位轴；
    /// - ⚠️ 判据层面有真实代价：`oauth` 的「成功页不得出现程序位名」那条断言
    ///   原先用 `Trae CN` 当标记，现在它同时是区域名 ⇒ 只能用 `TraeWork CN` /
    ///   `TraeCode CN` 当标记（见该用例的说明）。**这是本命名的已知成本。**
    ///
    /// ## ⚠️ 它**只用于 Trae 侧**
    ///
    /// 前端的**组合**标签（作用域芯片「平台 + 区域」，见 `scope-chip.tsx`）用的是
    /// **跨平台**的区域词（`shared.region.version.*`，WorkBuddy 也读它）。
    /// 把本值塞进那个组合会得到「Trae Trae CN」——
    /// 因此 Trae 侧另有一对独立词条（`trae.region.*`），本函数只服务 Trae 自己的文案。
    pub fn display_name(self) -> &'static str {
        match self {
            TraeRegion::Cn => "Trae CN",
            TraeRegion::Global => "Trae",
        }
    }

    /// 从字符串解析区域，**宽容**并兼容历史取值。
    ///
    /// ⚠️ 历史值 `trae_work` / `trae_cn` **都映射到 [`TraeRegion::Cn`]**：
    /// 它们是改造前的「产品线」标识，而那两个产品线**都是国内构建**。
    /// 老前端/老链接带着这两个值进来时必须落到国内版，**不能**落到国际版 ——
    /// 否则会在国际版里读一个空账号库，用户看到的是「账号凭空消失」。
    pub fn parse(s: &str) -> Option<TraeRegion> {
        match s.trim().to_ascii_lowercase().as_str() {
            "cn" | "china" | "domestic" | "trae_work" | "trae_cn" | "traework" | "trae" => {
                Some(TraeRegion::Cn)
            }
            "global" | "intl" | "international" | "sg" | "ai" => Some(TraeRegion::Global),
            _ => None,
        }
    }

    /// 全部区域（遍历用，顺序稳定）。
    pub fn all() -> [TraeRegion; 2] {
        [TraeRegion::Cn, TraeRegion::Global]
    }
}

impl Default for TraeRegion {
    /// 默认国内版：与改造前的默认变体（`trae_work`）落在**同一个账号库**上，
    /// 因此「不传 region」对老调用点的语义就是「行为与升级前一模一样」。
    fn default() -> Self {
        TraeRegion::Cn
    }
}

/// Trae 的程序位（**执行轴**）：登录态写进哪个客户端目录、启动哪个 exe。
///
/// ★ issue #28 收敛后**只有一个可选取值** [`TraeProgram::Trae`]（每区域一个，
/// 区域差异由 [`TraeRegion`] 承载）。`TraeWork` / `TraeCode` 是**退役的遗留标识**，
/// 只服务「解码老数据」与「标识迁移来源」，见模块头。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraeProgram {
    /// **退役**：TraeWork 程序线（客户端 `TRAE SOLO*`）。
    ///
    /// 自 2026-10-13 起客户端已把它**并入** [`TraeProgram::Trae`]，自 2026-10-15
    /// （issue #28）本程序位从所有「可选」表里**移除**：不在 [`all_program_specs`] /
    /// [`programs_of`] 里、不进界面、不参与探测与启动。
    ///
    /// ⚠️ **仍然保留**（不是死代码）：[Self::parse] 要认它的历史写法（老参数 / 老日志 /
    /// 老配置），[`super::snapshot_migrate`] 要靠它枚举**旧快照池目录**。
    TraeWork,
    /// **退役**：Trae 代码编辑器程序线（客户端 `Trae CN` / `Trae`）。
    ///
    /// 它与 [`TraeProgram::Trae`] **物理上是同一个客户端** —— 合并时复用了目录名与 exe 名，
    /// 所以收敛前两枚按钮指向同一个 exe，用户点哪一枚都「看起来能切」但界面上分不清。
    /// issue #28 收敛后只保留合并版一枚。保留本取值的理由与 [`Self::TraeWork`] 相同。
    TraeCode,
    /// **合并后的单一客户端**（issue #20；issue #28 起是**唯一**可选程序位）：CN `Trae CN` / 国际 `Trae`。
    ///
    /// 它是 TraeWork 与 TraeCode 两条产品线合并的产物，**继承**了 TraeCode 的目录名与
    /// exe 名 —— 这正是收敛的必要前提：合并版存在 ⇒ 遗留程序位可以退役。
    ///
    /// ## 依据（本机实测，不是推断）
    ///
    /// 客户端 2026-10-13 把 TraeWork 与 TraeCode 合并成一条产品线，并把旧数据迁进
    /// 合并后的客户端。证据是 `%APPDATA%\Trae CN\migration-job.json`（逐字）：
    ///
    /// ```json
    /// { "sourceDataPath": "…\\AppData\\Roaming\\TRAE SOLO CN",
    ///   "sourceInstallPath": "D:\\Programs\\TRAE SOLO CN",
    ///   "sourceAppName": "TRAE SOLO CN",
    ///   "targetDataPath": "…\\AppData\\Roaming\\Trae CN",
    ///   "removeApp": true, "removeData": false, "status": "completed" }
    /// ```
    ///
    /// 即：`TRAE SOLO CN`（TraeWork）被卸载、数据并入 `Trae CN`。
    ///
    /// 本取值是**面向用户的正确名字**：客户端现在就叫 `Trae`，不再叫 `TraeCode`。
    Trae,
}

impl TraeProgram {
    /// 稳定标识。
    pub fn as_str(self) -> &'static str {
        match self {
            TraeProgram::TraeWork => "trae_work",
            TraeProgram::TraeCode => "trae_code",
            TraeProgram::Trae => "trae",
        }
    }

    /// 从字符串解析程序线（宽容，接受客户端 exe / 目录名的常见写法）。
    pub fn parse(s: &str) -> Option<TraeProgram> {
        let lowered = s.trim().to_ascii_lowercase();
        let lowered = lowered.strip_suffix(".exe").unwrap_or(&lowered);
        if lowered.contains("solo") || lowered.contains("work") {
            return Some(TraeProgram::TraeWork);
        }
        // `code` 必须**先于**裸 `trae` 判定：`trae_code` 是遗留 IDE 程序位的标识，
        // 而裸 `trae` 自 issue #20 起指合并后的客户端 —— 两者是**不同槽位**
        // （虽然物理上同一个客户端），顺序写反会让 `trae_code` 落进 `Trae`。
        if lowered.contains("code") {
            return Some(TraeProgram::TraeCode);
        }
        // 裸 `trae`（含 `trae cn` / `trae cn.exe`）＝ 合并后的客户端。
        // 客户端自己的目录名/ exe 名就是这个，故它是最常见的输入。
        if lowered == "trae" || lowered == "trae cn" {
            return Some(TraeProgram::Trae);
        }
        // 其余含 `trae` 的历史写法（`trae_cn` / `global_trae_code` 已在上面的
        // `code` 分支处理）一律落遗留 TraeCode，**不猜**成合并版。
        if lowered.contains("trae") {
            return Some(TraeProgram::TraeCode);
        }
        None
    }

    /// 全部**可选程序位**（遍历用）。
    ///
    /// ★ issue #28 收敛后**只剩一条**：客户端已把 TraeWork / TraeCode 合并成一个，
    /// 每个区域只有「合并版」这一个程序位（区域差异由 [`TraeRegion`] 承载）。
    ///
    /// ⚠️ 本函数**不再**包含 `TraeWork` / `TraeCode` —— 那两条是退役的遗留标识
    /// （见模块头「每区域恰好一个」）。需要枚举**旧快照池**时走
    /// [`super::snapshot_migrate`] 的历史池名表，**不要**把遗留值塞回这里：
    /// 这个函数是「界面上有哪几枚按钮」的唯一事实来源。
    pub fn all() -> [TraeProgram; 1] {
        [TraeProgram::Trae]
    }
}

/// 一个**程序位**（区域 × 程序）的全部差异。
#[derive(Debug, Clone, Copy)]
pub struct ProgramSpec {
    /// 所属区域。
    pub region: TraeRegion,
    /// 所属程序线。
    pub program: TraeProgram,
    /// 界面展示名（**国内版带 ` CN` 后缀、国际版不带后缀**），见模块头「展示名」。
    pub display_name: &'static str,
    /// 客户端 `product.json` 的 `nameAlias`（官方自称，用于诊断与核对）。
    pub name_alias: &'static str,
    /// 客户端 `product.json` 的 `packageType`；未知时为 `None`（**不猜**）。
    pub package_type: Option<&'static str>,
    /// userData 目录名候选（`%APPDATA%\<名字>`）。**精确匹配，不做包含匹配**。
    pub data_dir_names: &'static [&'static str],
    /// 可执行文件名候选（精确匹配）。
    pub exe_names: &'static [&'static str],
    /// 进程名候选（Windows 精简名，不带 `.exe`）。
    pub proc_names: &'static [&'static str],
}

/// **合并后的 CN 客户端**（issue #20；issue #28 起是 CN 区域**唯一**的程序位）：客户端名 `Trae CN`。
///
/// ★ 它是**唯一**服务 CN 区域的 `ProgramSpec` —— 遗留的 `TraeWork CN` / `TraeCode CN`
/// 两条程序位已退役（见模块头），本表里不再有它们的位置。
const CN_TRAE: ProgramSpec = ProgramSpec {
    region: TraeRegion::Cn,
    program: TraeProgram::Trae,
    display_name: "Trae CN",
    // 客户端 `product.json` 的 `nameAlias` 实测仍是 `TraeCode CN`
    // （合并版 `D:\Programs\Trae CN\resources\app\product.json`，本机 2026-10-13 读取）
    // —— 上游没改这个字段，我们不替它改。
    name_alias: "TraeCode CN",
    package_type: Some("TRAE_CN"),
    data_dir_names: &["Trae CN"],
    exe_names: &["Trae CN.exe"],
    proc_names: &["Trae CN"],
};

/// **合并后的国际客户端**（issue #20；issue #28 起是国际区域**唯一**的程序位）：客户端名 `Trae`。
///
/// ★ **候选名由用户确认**（2026-10-13）：「国际版 `Trae` / `Trae.exe`、
/// 国内版 `Trae CN` / `Trae CN.exe`」—— 国内那份另有 `product.json` 实测佐证。
/// （规则：**国际版不带后缀**；国内版才带 ` CN`。）
const GLOBAL_TRAE: ProgramSpec = ProgramSpec {
    region: TraeRegion::Global,
    program: TraeProgram::Trae,
    display_name: "Trae",
    name_alias: "TraeCode（待实测）",
    package_type: None,
    data_dir_names: &["Trae"],
    exe_names: &["Trae.exe"],
    proc_names: &["Trae"],
};

/// 全部**可选程序位**（2 条 = 2 区域 × 1 程序），顺序稳定：先 CN、后国际。
///
/// ★ issue #28 收敛后**不再有重复候选名**：每个区域各一个程序位，`Trae CN` 与 `Trae`
/// 互不为子串（靠精确匹配区分）。
pub fn all_program_specs() -> [&'static ProgramSpec; 2] {
    [&CN_TRAE, &GLOBAL_TRAE]
}

/// 取某个程序位的描述符。
///
/// ## ★ 遗留程序位（`TraeWork` / `TraeCode`）返回**本区域的合并版**
///
/// 它们已退役（见模块头）：该区域只有一个程序位，而遗留标识仍要能被解码
/// （老参数、老日志、迁移来源）。返回 `None` 会把「已退役」降级成运行期判断，
/// 让每个调用点各写一份兜底；返回**本区域那一个** spec 则把事实收敛到一个落点：
/// **区域决定一切，程序轴只剩一个取值**。
pub fn program_spec(region: TraeRegion, program: TraeProgram) -> &'static ProgramSpec {
    let _ = program;
    match region {
        TraeRegion::Cn => &CN_TRAE,
        TraeRegion::Global => &GLOBAL_TRAE,
    }
}

/// 某个区域下的全部程序位（界面按区域渲染程序控件时遍历它）。
///
/// ★ issue #28 收敛后每区域**恰好一条** —— 这是「界面上有几枚程序按钮」的唯一事实来源。
pub fn programs_of(region: TraeRegion) -> [&'static ProgramSpec; 1] {
    match region {
        TraeRegion::Cn => [&CN_TRAE],
        TraeRegion::Global => [&GLOBAL_TRAE],
    }
}

/// 区域对应的端点集合。
///
/// **单一来源**：直接读 [`super::variant::variant_spec`] 的 CN / 国际端点表
/// （国内值已实测跑通、国际值取自国际版客户端自述），本模块不另立一份。
/// 取哪条变体都无所谓（两张 CN 变体的端点逐字相同，有单测钉住），取合并版最直白。
pub fn region_endpoints(region: TraeRegion) -> &'static EndpointSet {
    let spec = variant_spec(TraeVariant::TraeMerged);
    match region {
        TraeRegion::Cn => &spec.cn_endpoints,
        TraeRegion::Global => spec
            .global_endpoints
            .as_ref()
            .expect("国际版端点必须在表里登记（见 variant 模块的国际化端点单测）"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 区域标识与展示名稳定() {
        assert_eq!(TraeRegion::Cn.as_str(), "cn");
        assert_eq!(TraeRegion::Global.as_str(), "global");
        // ⚠️ 区域名 = **客户端自己的产品名**（用户 2026-10-13 定），
        // 与 `shared.region.version.*`（跨平台区域**词**）分工不同，见 `display_name` 的说明。
        // 反例（改坏会红）：退回「国内版 / 国际版」⇒ 前端 Trae 侧的区域名会与后端漂开。
        assert_eq!(TraeRegion::Cn.display_name(), "Trae CN");
        assert_eq!(TraeRegion::Global.display_name(), "Trae");
        // 默认必须落在与改造前同一份账号库上，否则老用户会看到账号凭空消失。
        assert_eq!(TraeRegion::default(), TraeRegion::Cn);
    }

    /// 历史取值 `trae_work` / `trae_cn` **必须都解析成国内版**：
    /// 它们是改造前的产品线标识，而两个产品线都是国内构建。
    /// 若谁把 `trae_cn` 解析成国际版，老链接会在国际版里读空账号库。
    #[test]
    fn 历史产品线标识一律解析为国内版() {
        for s in ["trae_work", "trae_cn", "CN", "trae", "cn"] {
            assert_eq!(TraeRegion::parse(s), Some(TraeRegion::Cn), "解析失败: {s}");
        }
        for s in ["global", "intl", "international", "SG"] {
            assert_eq!(
                TraeRegion::parse(s),
                Some(TraeRegion::Global),
                "解析失败: {s}"
            );
        }
        assert_eq!(TraeRegion::parse("doubao"), None);
    }

    #[test]
    fn 程序线解析接受客户端命名() {
        assert_eq!(
            TraeProgram::parse("TRAE SOLO CN.exe"),
            Some(TraeProgram::TraeWork)
        );
        assert_eq!(TraeProgram::parse("solo"), Some(TraeProgram::TraeWork));
        assert_eq!(TraeProgram::parse("trae_work"), Some(TraeProgram::TraeWork));
        // ⚠️ `Trae CN.exe` 自 issue #20 起落**合并版** `Trae` —— 客户端现在就叫这个名字，
        //    而遗留 TraeCode 与它**同名**（物理上同一个客户端），只能靠标识
        //    `trae_code` 区分。名字层面无法分辨两者，`parse` 取当前（合并版）为准。
        assert_eq!(TraeProgram::parse("Trae CN.exe"), Some(TraeProgram::Trae));
        assert_eq!(TraeProgram::parse("trae_code"), Some(TraeProgram::TraeCode));
        assert_eq!(TraeProgram::parse("doubao"), None);
    }

    /// ★ 裸 `trae` / `trae cn` 必须解析成**合并后的客户端**，而 `trae_code` 仍是遗留槽位
    /// （issue #20）。
    ///
    /// 反例（改坏会红）：把 `contains("trae")` 放在 `contains("code")` 之前 ——
    /// `trae_code` 会落进 `Trae`，于是「切到遗留 IDE 程序位」变成「切到合并版」。
    /// 两者物理上同一个客户端，故**不会报错**，只会让 `program` 标识静默漂移。
    #[test]
    fn 裸名trae解析为合并版而trae_code仍是遗留() {
        for s in ["trae", "Trae", "TRAE", "trae cn", "Trae CN", "Trae CN.exe"] {
            assert_eq!(
                TraeProgram::parse(s),
                Some(TraeProgram::Trae),
                "解析失败: {s}"
            );
        }
        for s in [
            "trae_code",
            "trae_code_global",
            "global_trae_code",
            "trae_cn",
            "traecode",
        ] {
            assert_eq!(
                TraeProgram::parse(s),
                Some(TraeProgram::TraeCode),
                "遗留标识必须落 TraeCode，不能被裸名规则抢走: {s}"
            );
        }
        // 退役标识仍能被 `parse` 认出（老数据解码）；**唯一可选**程序位的标识往返自洽。
        for program in TraeProgram::all() {
            assert_eq!(TraeProgram::parse(program.as_str()), Some(program));
        }
    }

    /// 展示名按客户端自己的写法：**国内版带 ` CN` 后缀、国际版不带后缀**。
    ///
    /// ★ issue #28 收敛后每区域只有一个程序位，名字就是客户端 `nameShort`
    /// （`Trae CN` / `Trae`）。
    #[test]
    fn 程序位展示名符合约定() {
        use TraeProgram::{Trae, TraeCode, TraeWork};
        use TraeRegion::{Cn, Global};
        assert_eq!(program_spec(Cn, Trae).display_name, "Trae CN");
        assert_eq!(program_spec(Global, Trae).display_name, "Trae");
        // ★ 遗留程序位**返回本区域的合并版**（它们已退役，见 `program_spec` 的说明）：
        //   反例（改坏会红）：让遗留标识返回 `None` 或另一张表 ⇒ 调用点各写一份兜底。
        assert_eq!(program_spec(Cn, TraeWork).display_name, "Trae CN");
        assert_eq!(program_spec(Cn, TraeCode).display_name, "Trae CN");
        assert_eq!(program_spec(Global, TraeWork).display_name, "Trae");
        assert_eq!(program_spec(Global, TraeCode).display_name, "Trae");
    }

    /// ★ 合并版客户端的候选名必须与**用户确认的写法**逐字一致（2026-10-13）：
    /// 「**国际版 `Trae` / `Trae.exe`、国内版 `Trae CN` / `Trae CN.exe`**」。
    ///
    /// ## 为什么必须钉住（而不是写在注释里）
    ///
    /// 候选名决定**探测结果**：它一漂，程序位就会指到错的客户端，而且**不报错**。
    /// 反例（改坏会红）：把国内那份写成裸 `Trae`（那是**国际版**客户端的名字）⇒
    /// 只装国际版客户端的机器上，国内程序位被判成「已安装」并指向国际版目录 ——
    /// 这正是 issue #3 修掉过的那一类缺陷（当时的现场是裸名 `Trae` 挂在国内 TraeCode 上）。
    ///
    /// 国内那份另有本机 `product.json` 实测佐证（`nameShort = "Trae CN"`、
    /// `applicationName = "trae-cn"`）；国际那份本机未装客户端，依据就是**用户确认**。
    #[test]
    fn 合并版候选名与用户确认的写法逐字一致() {
        use TraeProgram::Trae;
        use TraeRegion::{Cn, Global};
        for (region, dir_name, exe_name) in
            [(Cn, "Trae CN", "Trae CN.exe"), (Global, "Trae", "Trae.exe")]
        {
            let spec = program_spec(region, Trae);
            assert_eq!(
                spec.data_dir_names.to_vec(),
                vec![dir_name],
                "{region:?} 合并版的 userData 名漂了（用户确认的是 `{dir_name}`）"
            );
            assert_eq!(
                spec.exe_names.to_vec(),
                vec![exe_name],
                "{region:?} 合并版的 exe 名漂了（用户确认的是 `{exe_name}`）"
            );
            assert_eq!(
                spec.proc_names.to_vec(),
                vec![dir_name],
                "{region:?} 合并版的进程名漂了"
            );
        }
    }

    /// ★★ **每区域恰好一个程序位，且它就是合并版**（issue #28 收敛的不变式）。
    ///
    /// ## 为什么这条非钉不可
    ///
    /// `all_program_specs()` / `programs_of()` 是「界面上有几枚程序按钮」的**唯一事实来源**。
    /// 收敛前每区域有 3 枚（Work / Code / 合并版），其中两枚指向**已退役**的客户端
    /// ⇒ 用户点下去得到「未检测到 TraeWork CN」这类死路（issue #28 报障原文）。
    ///
    /// 反例（改坏会红）：把任意一条遗留程序位塞回 `all_program_specs()` ⇒ 立刻变红。
    #[test]
    fn 每区域恰好一个程序位且是合并版() {
        let specs = all_program_specs();
        assert_eq!(
            specs.len(),
            2,
            "前置：应恰有 2 条可选程序位（2 区域 × 1），否则本用例会恒真"
        );
        // 逐区域：恰好一条，且 `program` 是合并版、`region` 归属正确。
        for region in TraeRegion::all() {
            let of_region = programs_of(region);
            assert_eq!(
                of_region.len(),
                1,
                "{region:?} 应恰有 1 条程序位（收敛后遗留程序位已退役）"
            );
            assert_eq!(
                of_region[0].program,
                TraeProgram::Trae,
                "{region:?} 的唯一程序位必须是合并版"
            );
            assert_eq!(of_region[0].region, region, "{region:?} 的程序位归属漂了");
            assert!(
                specs
                    .iter()
                    .any(|s| s.region == region && s.program == TraeProgram::Trae),
                "{region:?} 的合并版程序位不在全量表里"
            );
        }
        // 两个区域**不能**共用候选名（差一个 ` CN` 后缀也算不同）。
        let cn = program_spec(TraeRegion::Cn, TraeProgram::Trae);
        let global = program_spec(TraeRegion::Global, TraeProgram::Trae);
        assert_ne!(cn.data_dir_names, global.data_dir_names);
        assert_ne!(cn.exe_names, global.exe_names);
        assert_ne!(cn.display_name, global.display_name);
    }

    /// ★★ 遗留程序位（`TraeWork` / `TraeCode`）**不再是可选程序位**，但仍要能解码。
    ///
    /// 它们只服务两件事（见模块头）：解码老参数 / 老日志，以及标识迁移来源。
    /// 因此：
    /// - `TraeProgram::all()` **不含**它们（那是「可选程序位」的口径）；
    /// - `TraeProgram::parse` 仍认它们的历史写法（老配置里就有）。
    ///
    /// 反例（改坏会红）：把 `TraeWork` 加回 `all()` ⇒ 界面上又会多出一枚死按钮。
    #[test]
    fn 遗留程序位退役但仍可解码() {
        let selectable = TraeProgram::all();
        assert_eq!(selectable.len(), 1, "可选程序位应只剩合并版一条");
        assert_eq!(selectable[0], TraeProgram::Trae);
        for retired in [TraeProgram::TraeWork, TraeProgram::TraeCode] {
            assert!(
                !selectable.contains(&retired),
                "{retired:?} 是退役程序位，不得出现在可选列表里"
            );
            // 退役 ≠ 认不出来：历史写法仍要能解析（迁移来源 / 老日志解码）。
            assert_eq!(
                TraeProgram::parse(retired.as_str()),
                Some(retired),
                "{retired:?} 的落盘标识必须仍能往返（老数据要能解码）"
            );
        }
        // `as_str` 往返对**全部**取值成立（含退役值）。
        for program in [
            TraeProgram::Trae,
            TraeProgram::TraeWork,
            TraeProgram::TraeCode,
        ] {
            assert_eq!(TraeProgram::parse(program.as_str()), Some(program));
        }
    }

    /// 程序位的候选名（目录 / exe / 进程）**必须两两互不重叠**。
    ///
    /// 这是本模块存在的理由之一：改造前国内/国外候选名混在一张表里按序取首个，
    /// 用包含关系匹配就会「装了 A、读了 B 的目录」。谁把它们合并回去，这里会红。
    ///
    /// ★ issue #28 收敛后**没有任何登记在案的例外**（收敛前 `Trae` × `TraeCode`
    /// 候选名逐字相同，那是「同一个客户端两条程序位」的投影；现在只剩一条）。
    /// ⇒ 这条断言从「白名单比对」升级为**零重叠**。
    #[test]
    fn 两个区域程序位候选名互不重叠() {
        let specs = all_program_specs();
        let mut overlaps: Vec<String> = Vec::new();
        for (i, a) in specs.iter().enumerate() {
            for b in specs.iter().skip(i + 1) {
                for (kind, left, right) in [
                    ("userData 目录名", a.data_dir_names, b.data_dir_names),
                    ("exe 名", a.exe_names, b.exe_names),
                    ("进程名", a.proc_names, b.proc_names),
                ] {
                    for name in left {
                        if right.contains(name) {
                            overlaps.push(format!(
                                "{kind}:{name}:{:?}/{:?}:{:?}/{:?}",
                                a.region, a.program, b.region, b.program
                            ));
                        }
                    }
                }
            }
        }
        assert!(
            overlaps.is_empty(),
            "两个区域的程序位候选名不得重叠（会互相抢）：{overlaps:?}"
        );
        // 前置：确实比对了对象，否则本用例恒真。
        assert_eq!(specs.len(), 2);
    }

    /// `programs_of` 必须给出该区域的**唯一**程序位（合并版），且它归属本区域。
    #[test]
    fn 按区域取程序位() {
        for region in TraeRegion::all() {
            let got = programs_of(region);
            assert_eq!(got.len(), 1, "每个区域应恰有 1 条程序位（合并版 Trae）");
            for spec in got {
                assert_eq!(spec.region, region, "程序位归属漂了: {:?}", spec.program);
            }
            assert_eq!(got[0].program, TraeProgram::Trae);
        }
    }

    /// 区域端点是**单一来源**：国内必须等于已实测的 CN 值，国际必须等于
    /// 国际版客户端自述值，且两者主机全不同（混用会把请求打到错的域）。
    #[test]
    fn 区域端点取自既有端点表() {
        let cn = region_endpoints(TraeRegion::Cn);
        assert_eq!(cn.account_base, "https://api.trae.cn");
        assert_eq!(cn.icube_base, "https://api.trae.com.cn");

        let global = region_endpoints(TraeRegion::Global);
        assert_eq!(global.account_base, "https://grow-normal.trae.ai");
        // ExchangeToken 实际落在 `ug-normal.trae.ai`（线上 404 探测修正，
        // 客户端自述的 `icube-normal.trae.ai` 两条路径都 404）。
        assert_eq!(global.icube_base, "https://ug-normal.trae.ai");
        assert_ne!(global.account_base, cn.account_base);

        // 授权页域（`bootConfig.consoleHost`）同样按区域分家 —— 它不是 API 域，
        // 但漏了分家的后果更隐蔽：国际版用户在**国内**授权页上登录，
        // 页面走完也不回调，症状与「回调端口没监听」几乎一样。
        assert_eq!(cn.console_base, "https://www.trae.cn");
        assert_eq!(global.console_base, "https://www.trae.ai");
        assert_ne!(global.console_base, cn.console_base);
    }
}
