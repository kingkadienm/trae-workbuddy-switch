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
//! ## 程序位（program instance）与其官方名
//!
//! 取值来自**两处权威来源**（本机 2026-09-21 实测），不是我们起的名字：
//!
//! - 客户端 `<安装根>\resources\app\product.json` 的 `nameAlias` / `packageType`；
//! - 系统注册表 `Uninstall` 项的 `DisplayName`（用户在「应用和功能」里看到的名字）。
//!
//! | 区域 | 程序 | userData 目录 | exe / 进程 | `nameAlias` | `packageType` | 注册表显示名 |
//! |:---|:---|:---|:---|:---|:---|:---|
//! | CN | TraeWork | `TRAE SOLO CN` | `TRAE SOLO CN.exe` | `TraeWork CN` | `SOLO_CN` | TraeWork CN (User) |
//! | CN | TraeCode | `Trae CN` | `Trae CN.exe` | `TraeCode CN` | `TRAE_CN` | TraeCode CN (User) |
//! | Global | TraeWork | `TRAE SOLO` | `TRAE SOLO.exe` | `TraeWork` | `SOLO_I18N` | TraeWork (User) |
//! | Global | TraeCode | `Trae` | `Trae.exe` | `TraeCode`（待实测） | `TRAE_I18N`（推断，见下） | 本机未安装 |
//!
//! ⚠️ **候选名必须互不重叠**：`TRAE SOLO CN` 与 `TRAE SOLO` 只差一个后缀，
//! `Trae CN` 与 `Trae` 是包含关系。用「包含匹配」判程序位会互相抢
//! （出现「装了 A、读了 B 的目录」），因此 [`ProgramSpec`] 的候选名一律
//! **精确逐字比对**，不重叠由单测钉住。
//!
//! ## 四个程序位是否都真的接上了（2026-09-28，issue #3）
//!
//! 本模块一直有 4 个 [`ProgramSpec`]，但**第 4 个（Global × TraeCode）曾经接不上**：
//! `platform::variant_for_program` 对它返回 `None`，于是界面上那一格永远是
//! 「未检测到」。issue #3 报的「不支持 TraeCode」就是这一格。
//!
//! 现在 4 个程序位都映射到 [`super::variant::TraeVariant`] 的一个取值
//! （`TraeWork` / `Trae` / `Global` / `GlobalTraeCode`），探测、启动、切换、
//! 登录态快照四件事因此都按程序位分家。判据见
//! `platform::tests::四个程序位全部已建模且自洽`。
//!
//! ⚠️ 唯一**推断**（非实测）的是 `TraeCode` 国际版的 `packageType`：本机未装该客户端。
//! 它只被 [`super::variant::OAuthLine::from_package_type`] 消费，而那个分派
//! **只区分 `SOLO_*` 与非 `SOLO_*`** ⇒ 任何非 `SOLO_*` 取值都落 TRAE 线（IDE 该用的
//! 那条钥匙），所以推断错也**不会**把请求打到错的域或换错钥匙。
//!
//! ## 展示名：区域前缀 + 程序名
//!
//! 程序位的展示名按用户确认的写法（国际版带 `AI` 后缀，国内版不带）：
//! `TraeWork` / `TraeCode` / `TraeWork AI` / `Trae AI`。
//! 区域前缀（[`TraeRegion::display_name`]，`国内版` / `国际版`）由调用方按需拼接 ——
//! 界面上是「国内版 TraeWork」「国际版 TraeWork AI」，日志里也能拼成无歧义的全名。
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

    /// 界面上的区域前缀（与 WorkBuddy 的「国内版 / 国际版」同形）。
    pub fn display_name(self) -> &'static str {
        match self {
            TraeRegion::Cn => "国内版",
            TraeRegion::Global => "国际版",
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

/// Trae 的两条程序线。
///
/// 它是**执行轴**：只决定「登录态写进哪个客户端目录、启动哪个 exe」，
/// 不参与账号归属。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraeProgram {
    /// TraeWork 程序线（客户端 `TRAE SOLO*`，官方别名 `TraeWork*`）。
    TraeWork,
    /// Trae 代码编辑器程序线（客户端 `Trae*`，官方别名 `TraeCode*`）。
    TraeCode,
}

impl TraeProgram {
    /// 稳定标识。
    pub fn as_str(self) -> &'static str {
        match self {
            TraeProgram::TraeWork => "trae_work",
            TraeProgram::TraeCode => "trae_code",
        }
    }

    /// 从字符串解析程序线（宽容，接受客户端 exe / 目录名的常见写法）。
    pub fn parse(s: &str) -> Option<TraeProgram> {
        let lowered = s.trim().to_ascii_lowercase();
        let lowered = lowered.strip_suffix(".exe").unwrap_or(&lowered);
        if lowered.contains("solo") || lowered.contains("work") {
            return Some(TraeProgram::TraeWork);
        }
        if lowered.contains("trae") || lowered.contains("code") {
            return Some(TraeProgram::TraeCode);
        }
        None
    }

    /// 全部程序线（遍历用，顺序稳定）。
    pub fn all() -> [TraeProgram; 2] {
        [TraeProgram::TraeWork, TraeProgram::TraeCode]
    }
}

/// 一个**程序位**（区域 × 程序）的全部差异。
#[derive(Debug, Clone, Copy)]
pub struct ProgramSpec {
    /// 所属区域。
    pub region: TraeRegion,
    /// 所属程序线。
    pub program: TraeProgram,
    /// 界面展示名（国际版带 `AI` 后缀），见模块头「展示名」。
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

const CN_TRAE_WORK: ProgramSpec = ProgramSpec {
    region: TraeRegion::Cn,
    program: TraeProgram::TraeWork,
    display_name: "TraeWork",
    name_alias: "TraeWork CN",
    package_type: Some("SOLO_CN"),
    data_dir_names: &["TRAE SOLO CN"],
    exe_names: &["TRAE SOLO CN.exe"],
    proc_names: &["TRAE SOLO CN"],
};

const CN_TRAE_CODE: ProgramSpec = ProgramSpec {
    region: TraeRegion::Cn,
    program: TraeProgram::TraeCode,
    display_name: "TraeCode",
    name_alias: "TraeCode CN",
    package_type: Some("TRAE_CN"),
    data_dir_names: &["Trae CN"],
    exe_names: &["Trae CN.exe"],
    proc_names: &["Trae CN"],
};

const GLOBAL_TRAE_WORK: ProgramSpec = ProgramSpec {
    region: TraeRegion::Global,
    program: TraeProgram::TraeWork,
    display_name: "TraeWork AI",
    name_alias: "TraeWork",
    package_type: Some("SOLO_I18N"),
    data_dir_names: &["TRAE SOLO"],
    exe_names: &["TRAE SOLO.exe"],
    proc_names: &["TRAE SOLO"],
};

/// 国际版 TraeCode：**本机未安装**，`nameAlias` / `packageType` 无从实测，
/// 因此 `package_type` 留 `None`、别名按同族命名规则推断并在此显式标注「待实测」。
/// 猜测只影响诊断文案；候选名（目录 / exe / 进程）是客户端通用命名，风险可控。
///
/// ⚠️ 它与 [`super::variant::GLOBAL_TRAE_CODE_SPEC`] 的 `package_type` **不是同一件事**：
/// 那边的字段是 `&'static str`（授权页分派必须要一个具体值），填的是同一推断值
/// `TRAE_I18N`；本字段是**诊断用**、允许诚实留空。两处都写了「未实测」，
/// 谁先拿到真实客户端就一起更正。
const GLOBAL_TRAE_CODE: ProgramSpec = ProgramSpec {
    region: TraeRegion::Global,
    program: TraeProgram::TraeCode,
    display_name: "Trae AI",
    name_alias: "TraeCode（待实测）",
    package_type: None,
    data_dir_names: &["Trae"],
    exe_names: &["Trae.exe"],
    proc_names: &["Trae"],
};

/// 全部程序位（4 个），顺序稳定：先区域、后程序线。
pub fn all_program_specs() -> [&'static ProgramSpec; 4] {
    [
        &CN_TRAE_WORK,
        &CN_TRAE_CODE,
        &GLOBAL_TRAE_WORK,
        &GLOBAL_TRAE_CODE,
    ]
}

/// 取某个程序位的描述符。
pub fn program_spec(region: TraeRegion, program: TraeProgram) -> &'static ProgramSpec {
    match (region, program) {
        (TraeRegion::Cn, TraeProgram::TraeWork) => &CN_TRAE_WORK,
        (TraeRegion::Cn, TraeProgram::TraeCode) => &CN_TRAE_CODE,
        (TraeRegion::Global, TraeProgram::TraeWork) => &GLOBAL_TRAE_WORK,
        (TraeRegion::Global, TraeProgram::TraeCode) => &GLOBAL_TRAE_CODE,
    }
}

/// 某个区域下的全部程序位（界面按区域渲染程序控件时遍历它）。
pub fn programs_of(region: TraeRegion) -> [&'static ProgramSpec; 2] {
    match region {
        TraeRegion::Cn => [&CN_TRAE_WORK, &CN_TRAE_CODE],
        TraeRegion::Global => [&GLOBAL_TRAE_WORK, &GLOBAL_TRAE_CODE],
    }
}

/// 区域对应的端点集合。
///
/// **单一来源**：直接读 [`super::variant::variant_spec`] 的 CN / 国际端点表
/// （国内值已实测跑通、国际值取自国际版客户端自述），本模块不另立一份。
pub fn region_endpoints(region: TraeRegion) -> &'static EndpointSet {
    let spec = variant_spec(TraeVariant::TraeWork);
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
        assert_eq!(TraeRegion::Cn.display_name(), "国内版");
        assert_eq!(TraeRegion::Global.display_name(), "国际版");
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
            assert_eq!(TraeRegion::parse(s), Some(TraeRegion::Global), "解析失败: {s}");
        }
        assert_eq!(TraeRegion::parse("doubao"), None);
    }

    #[test]
    fn 程序线解析接受客户端命名() {
        assert_eq!(TraeProgram::parse("TRAE SOLO CN.exe"), Some(TraeProgram::TraeWork));
        assert_eq!(TraeProgram::parse("solo"), Some(TraeProgram::TraeWork));
        assert_eq!(TraeProgram::parse("trae_work"), Some(TraeProgram::TraeWork));
        assert_eq!(TraeProgram::parse("Trae CN.exe"), Some(TraeProgram::TraeCode));
        assert_eq!(TraeProgram::parse("trae_code"), Some(TraeProgram::TraeCode));
        assert_eq!(TraeProgram::parse("doubao"), None);
    }

    /// 展示名按用户确认的写法：国际版带 `AI`，国内版不带。
    #[test]
    fn 程序位展示名符合约定() {
        use TraeProgram::{TraeCode, TraeWork};
        use TraeRegion::{Cn, Global};
        assert_eq!(program_spec(Cn, TraeWork).display_name, "TraeWork");
        assert_eq!(program_spec(Cn, TraeCode).display_name, "TraeCode");
        assert_eq!(program_spec(Global, TraeWork).display_name, "TraeWork AI");
        assert_eq!(program_spec(Global, TraeCode).display_name, "Trae AI");
    }

    /// 四个程序位的候选名（目录 / exe / 进程）**必须两两互不重叠**。
    ///
    /// 这是本模块存在的理由之一：改造前国内/国外候选名混在一张表里按序取首个，
    /// 用包含关系匹配就会「装了 A、读了 B 的目录」。谁把它们合并回去，这里会红。
    #[test]
    fn 四个程序位的候选名互不重叠() {
        let specs = all_program_specs();
        for (i, a) in specs.iter().enumerate() {
            for b in specs.iter().skip(i + 1) {
                let label = format!("{:?}/{:?} 与 {:?}/{:?}", a.region, a.program, b.region, b.program);
                for name in a.data_dir_names {
                    assert!(!b.data_dir_names.contains(name), "userData 目录名重叠: {name}（{label}）");
                }
                for name in a.exe_names {
                    assert!(!b.exe_names.contains(name), "exe 名重叠: {name}（{label}）");
                }
                for name in a.proc_names {
                    assert!(!b.proc_names.contains(name), "进程名重叠: {name}（{label}）");
                }
            }
        }
        // 反向：CN 与 Global 的同名程序**不能**共用候选名（差一个后缀也算）。
        assert_ne!(
            program_spec(TraeRegion::Cn, TraeProgram::TraeWork).data_dir_names,
            program_spec(TraeRegion::Global, TraeProgram::TraeWork).data_dir_names
        );
    }

    /// `programs_of` 必须给出该区域的两条程序线，且每条都归属本区域。
    #[test]
    fn 按区域取程序位() {
        for region in TraeRegion::all() {
            let got = programs_of(region);
            assert_eq!(got.len(), 2);
            for spec in got {
                assert_eq!(spec.region, region, "程序位归属漂了: {:?}", spec.program);
            }
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
