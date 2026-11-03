//! Interface translations. Command ids and document text stay stable; only interface chrome is
//! translated. Untranslated labels fall back to English, so coverage can grow incrementally.
//!
//! The interface language is a preference ([`crate::UiState::language`]): on first run it defaults
//! to the system's language ([`Language::system`]), and the View tab switches it by hand.
//!
//! CJK fonts stay system-provided: [`crate::theme`] appends available CJK faces to every family so
//! Japanese, Chinese and Korean labels and workbook text can render without bundling font files.

use serde::{Deserialize, Serialize};

mod cs;
mod de;
mod el;
mod es;
mod es_functions;
mod fr;
mod it;
mod ja;
mod ko;
mod nl;
mod pl;
mod pt;
mod ro;
mod ru;
mod sr;
mod tr;
mod uk;
mod vi;
mod zh;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    En,
    Es,
    El,
    Zh,
    Ja,
    Ko,
    Ru,
    #[serde(rename = "pt")]
    PtBr,
    De,
    Cs,
    Fr,
    Sr,
    Tr,
    Pl,
    It,
    Uk,
    Vi,
    Nl,
    Ro,
}

impl Language {
    pub const ALL: [Self; 19] = [
        Self::En,
        Self::Es,
        Self::El,
        Self::Zh,
        Self::Ja,
        Self::Ko,
        Self::Ru,
        Self::PtBr,
        Self::De,
        Self::Cs,
        Self::Fr,
        Self::Sr,
        Self::Pl,
        Self::It,
        Self::Uk,
        Self::Vi,
        Self::Nl,
        Self::Ro,
        Self::Tr,
    ];

    /// Every language in the order the switcher lists them: English first (the source language),
    /// then the rest by their own name with accents folded, so Čeština sits under C. Names in
    /// other scripts follow the Latin ones in code point order: Cyrillic, then the CJK languages.
    pub fn for_picker() -> Vec<Self> {
        let mut rest: Vec<Self> = Self::ALL.into_iter().filter(|l| *l != Self::En).collect();
        rest.sort_by_cached_key(|l| name_sort_key(l.name()));
        std::iter::once(Self::En).chain(rest).collect()
    }

    /// The language's own name, shown in the switcher.
    pub fn name(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::Es => "Español",
            Self::El => "Ελληνικά",
            Self::Zh => "简体中文",
            Self::Ja => "日本語",
            Self::Ko => "한국어",
            Self::Ru => "Русский",
            Self::PtBr => "Português (Brasil)",
            Self::De => "Deutsch",
            Self::Cs => "Čeština",
            Self::Fr => "Français",
            Self::Sr => "Srpski",
            Self::Tr => "Türkçe",
            Self::Pl => "Polski",
            Self::It => "Italiano",
            Self::Uk => "Українська",
            Self::Vi => "Tiếng Việt",
            Self::Nl => "Nederlands",
            Self::Ro => "Română",
        }
    }

    /// The language's code, as saved in `ui.json` and taken by `app.language.set`.
    pub const fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Es => "es",
            Self::El => "el",
            Self::Zh => "zh",
            Self::Ja => "ja",
            Self::Ko => "ko",
            Self::Ru => "ru",
            Self::PtBr => "pt",
            Self::De => "de",
            Self::Cs => "cs",
            Self::Fr => "fr",
            Self::Sr => "sr",
            Self::Tr => "tr",
            Self::Pl => "pl",
            Self::It => "it",
            Self::Uk => "uk",
            Self::Vi => "vi",
            Self::Nl => "nl",
            Self::Ro => "ro",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        Self::from_tag(code)
    }

    /// Every code `app.language.set` takes, quoted and joined (`"en"|"es"|…`), for command docs
    /// and errors.
    pub fn code_list(separator: &str) -> String {
        Self::ALL.iter().map(|l| format!("\"{}\"", l.code())).collect::<Vec<_>>().join(separator)
    }

    fn from_tag(tag: &str) -> Option<Self> {
        match tag.trim().split(['-', '_']).next()?.to_ascii_lowercase().as_str() {
            "en" => Some(Self::En),
            "es" => Some(Self::Es),
            "el" => Some(Self::El),
            "zh" => Some(Self::Zh),
            "ja" => Some(Self::Ja),
            "ko" => Some(Self::Ko),
            "ru" => Some(Self::Ru),
            "pt" => Some(Self::PtBr),
            "de" => Some(Self::De),
            "cs" => Some(Self::Cs),
            "fr" => Some(Self::Fr),
            "sr" => Some(Self::Sr),
            "tr" => Some(Self::Tr),
            "pl" => Some(Self::Pl),
            "it" => Some(Self::It),
            "uk" => Some(Self::Uk),
            "vi" => Some(Self::Vi),
            "nl" => Some(Self::Nl),
            "ro" => Some(Self::Ro),
            _ => None,
        }
    }

    /// The system's preferred interface language, best effort; English when it is unknown or
    /// unsupported. Used as the first-run default while nothing is saved.
    pub fn system() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            sys_locale::get_locale().and_then(|tag| Self::from_tag(&tag)).unwrap_or(Self::En)
        }
        #[cfg(target_arch = "wasm32")]
        {
            web_sys::window().and_then(|window| window.navigator().language()).and_then(|tag| Self::from_tag(&tag)).unwrap_or(Self::En)
        }
    }

    pub fn formula_locale(self) -> gridcraft_engine::formula::FormulaLocale {
        match self {
            Self::Es => gridcraft_engine::formula::FormulaLocale::Es,
            Self::PtBr => gridcraft_engine::formula::FormulaLocale::Pt,
            _ => gridcraft_engine::formula::FormulaLocale::En,
        }
    }

    /// Translates an interface label; untranslated labels fall back to English.
    ///
    /// A label that needs two translations (Home's text "Orientation" vs Page Layout's print
    /// "Orientation") is keyed `context|label`: English shows only the part after the `|`.
    pub fn tr(self, text: &str) -> &str {
        let translated = match self {
            Self::En => None,
            Self::Es => es::translate(text),
            Self::El => el::translate(text),
            Self::Ja => ja::translate(text),
            Self::Zh => zh::translate(text),
            Self::Ko => ko::translate(text),
            Self::Ru => ru::translate(text),
            Self::PtBr => pt::translate(text),
            Self::De => de::translate(text),
            Self::Cs => cs::translate(text),
            Self::Fr => fr::translate(text),
            Self::Sr => sr::translate(text),
            Self::Tr => tr::translate(text),
            Self::Pl => pl::translate(text),
            Self::It => it::translate(text),
            Self::Uk => uk::translate(text),
            Self::Vi => vi::translate(text),
            Self::Nl => nl::translate(text),
            Self::Ro => ro::translate(text),
        };
        translated.unwrap_or_else(|| text.rsplit_once('|').map_or(text, |(_, label)| label))
    }

    pub(crate) fn tr_error(self, message: &str) -> String {
        let message = if message.starts_with("command `") { crate::clean_error(message) } else { message.to_owned() };

        let translated = self.tr(&message);
        let fallback = message.rsplit_once('|').map_or(message.as_str(), |(_, label)| label);

        if translated == fallback { message } else { translated.to_owned() }
    }

    /// A worksheet function's description in this language, looked up by the English text of the
    /// function catalog; `None` when untranslated, and the help shows the English.
    pub fn function_description(self, english: &str) -> Option<&'static str> {
        lookup(self.function_help()?.0, english)
    }

    /// A function argument name in this language, given as the catalog spells it without
    /// brackets or index (`number`, `logical_test`).
    pub fn function_argument(self, english: &str) -> Option<&'static str> {
        lookup(self.function_help()?.1, english)
    }

    /// The function help tables (descriptions, argument names) of the languages that have them.
    fn function_help(self) -> Option<(HelpTable, HelpTable)> {
        match self {
            Self::Es => Some((es_functions::DESCRIPTIONS, es_functions::ARGUMENTS)),
            _ => None,
        }
    }
}

/// English text and its translation, sorted by the English.
type HelpTable = &'static [(&'static str, &'static str)];

/// The translation of `english` in a help table (binary search, so the table must stay sorted).
fn lookup(table: HelpTable, english: &str) -> Option<&'static str> {
    let i = table.binary_search_by(|(key, _)| (*key).cmp(english)).ok()?;
    table.get(i).map(|(_, translated)| *translated)
}

/// Deserializes a saved language leniently: an unknown or malformed value (a language removed in a
/// later version, a hand-edited `ui.json`) reads as English instead of failing the whole `UiState`,
/// which would reset every other preference with it.
pub fn lenient<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Language, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Code(String),
        Other(serde::de::IgnoredAny),
    }
    Ok(match Raw::deserialize(d)? {
        Raw::Code(code) => Language::parse(&code).unwrap_or_default(),
        Raw::Other(_) => Language::default(),
    })
}

/// The language a saved `ui.json` asks for, if it names a supported one. The desktop app uses it
/// to tell "saved English" from "nothing usable saved" (first run, an older file, an unknown
/// value), which starts from [`Language::system`] instead.
pub fn saved_language(ui_json: &serde_json::Value) -> Option<Language> {
    ui_json.get("language").and_then(serde_json::Value::as_str).and_then(Language::parse)
}

/// egui context-data key holding the language of the frame being drawn. Widgets that only get a
/// `Ui` (not the app) read it to translate their labels; [`set_current`] writes it once per frame.
fn lang_key() -> egui::Id {
    egui::Id::unique("gridcraft.language")
}

/// The language the current frame is drawing in. `En` when unset.
pub fn current(ctx: &egui::Context) -> Language {
    ctx.data(|d| d.get_temp(lang_key())).unwrap_or_default()
}

/// Sets the language for the current frame; call once per frame before anything is drawn.
pub fn set_current(ctx: &egui::Context, language: Language) {
    ctx.data_mut(|d| d.insert_temp(lang_key(), language));
}

/// English label → Japanese. Keys are the exact interface strings (command ids, file names and
/// document text are never translated); anything absent falls back to English.
const JAPANESE: &[(&str, &str)] = &[
    ("Paste", "貼り付け"),
    ("Paste Values", "値の貼り付け"),
    ("Paste Formulas", "数式の貼り付け"),
    ("Paste Formatting", "書式の貼り付け"),
    ("Transpose", "行列を入れ替え"),
    ("Paste Link", "リンクの貼り付け"),
    ("Paste Special…", "形式を選択して貼り付け…"),
    ("Insert Copied Cells", "コピーしたセルの挿入"),
    ("Insert Copied Cells…", "コピーしたセルの挿入…"),
    ("Insert Cut Cells", "切り取ったセルの挿入"),
    ("Insert Paste", "挿入貼り付け"),
    ("Cut (⌘X)", "切り取り (⌘X)"),
    ("Copy (⌘C)", "コピー (⌘C)"),
    ("Format Painter (double-click to keep it on)", "書式のコピー/貼り付け (ダブルクリックで固定)"),
    ("Bold (⌘B)", "太字 (⌘B)"),
    ("Italic (⌘I)", "斜体 (⌘I)"),
    ("Underline (⌘U)", "下線 (⌘U)"),
    ("Double Underline", "二重下線"),
    ("Strikethrough", "取り消し線"),
    ("Underline", "下線"),
    ("Increase Font Size", "フォントの拡大"),
    ("Decrease Font Size", "フォントの縮小"),
    ("Wrap Text", "折り返して全体を表示"),
    ("Merge & Center", "セルを結合して中央揃え"),
    ("Merge Cells", "セルの結合"),
    ("Merge Across", "横方向に結合"),
    ("Unmerge Cells", "セル結合の解除"),
    ("Fill Color", "塗りつぶしの色"),
    ("Font Color", "フォントの色"),
    ("Borders", "罫線"),
    ("Top\nAlign", "上揃え"),
    ("Middle\nAlign", "中央揃え (縦)"),
    ("Bottom\nAlign", "下揃え"),
    ("Left", "左"),
    ("Right", "右"),
    ("Center", "中央"),
    ("Horizontal", "水平"),
    ("Vertical Text", "縦書き"),
    ("Decrease Indent", "インデントを減らす"),
    ("Increase Indent", "インデントを増やす"),
    ("Orientation", "方向"),
    ("Page Layout|Orientation", "印刷の向き"),
    ("Angle Counterclockwise", "左回りに回転"),
    ("Angle Clockwise", "右回りに回転"),
    ("Rotate Text Up", "文字列を上に回転"),
    ("Rotate Text Down", "文字列を下に回転"),
    ("Accounting Number Format", "通貨表示形式"),
    ("Percent Style", "パーセント表示"),
    ("Comma Style", "桁区切りスタイル"),
    ("Increase Decimal", "小数点以下の桁数を増やす"),
    ("Decrease Decimal", "小数点以下の桁数を減らす"),
    ("More Accounting Formats…", "その他の通貨表示形式…"),
    ("Conditional\nFormatting", "条件付き書式"),
    ("Conditional Formatting", "条件付き書式"),
    ("Format\nas Table", "テーブルとして書式設定"),
    ("Cell\nStyles", "セルのスタイル"),
    ("Format Cells…", "セルの書式設定…"),
    ("Format Cell Alignment…", "セルの配置…"),
    ("Format\nPane", "書式ウィンドウ"),
    ("Format", "書式"),
    ("Insert", "挿入"),
    ("Insert Cells…", "セルの挿入…"),
    ("Delete", "削除"),
    ("Delete Cells…", "セルの削除…"),
    ("Insert Sheet Rows", "シートの行を挿入"),
    ("Insert Sheet Columns", "シートの列を挿入"),
    ("Delete Sheet Rows", "シートの行を削除"),
    ("Delete Sheet Columns", "シートの列を削除"),
    ("Row Height…", "行の高さ…"),
    ("AutoFit Row Height", "行の高さを自動調整"),
    ("Column Width…", "列の幅…"),
    ("AutoFit Column Width", "列の幅を自動調整"),
    ("Default Width…", "標準の幅…"),
    ("Hide Rows", "行を表示しない"),
    ("Unhide Rows", "再表示"),
    ("Hide Columns", "列の表示/非表示"),
    ("Unhide Columns", "再表示"),
    ("Lock Cell", "セルのロック"),
    ("Clear", "クリア"),
    ("Clear All", "すべてクリア"),
    ("Clear Formats", "書式のクリア"),
    ("Clear Contents", "数式と値のクリア"),
    ("Clear Comments and Notes", "コメントとメモのクリア"),
    ("Clear Hyperlinks", "ハイパーリンクのクリア"),
    ("AutoSum", "オート SUM"),
    ("AutoSum (⌘⇧T)", "オート SUM (⌘⇧T)"),
    ("Flash\nFill", "フラッシュ フィル"),
    ("Flash Fill", "フラッシュ フィル"),
    ("Fill", "フィル"),
    ("Clear Print Area", "印刷範囲のクリア"),
    ("Find &\nSelect", "検索と選択"),
    ("Find…", "検索…"),
    ("Replace…", "置換…"),
    ("Go To…", "ジャンプ…"),
    ("Go To Special…", "セルを選択…"),
    ("PivotTable", "ピボットテーブル"),
    ("Recommended\nCharts", "推奨グラフ"),
    ("Table", "テーブル"),
    ("Pictures", "画像"),
    ("Shapes", "図形"),
    ("Icons", "アイコン"),
    ("Text\nBox", "テキスト ボックス"),
    ("Comment", "コメント"),
    ("New\nComment", "新しいコメント"),
    ("New Note", "新しいメモ"),
    ("Show/Hide Note", "メモの表示/非表示"),
    ("Header &\nFooter", "ヘッダーとフッター"),
    ("Text to\nColumns", "区切り位置"),
    ("Link", "リンク"),
    ("Symbol", "記号"),
    ("Sparklines", "スパークライン"),
    ("Checkbox", "チェックボックス"),
    ("Insert checkboxes in the selected cells", "選択したセルにチェックボックスを挿入"),
    ("Header & Footer", "ヘッダーとフッター"),
    ("Draw with ink", "インクで描画"),
    ("Eraser", "消しゴム"),
    ("Ink to\nShape", "インクを図形に変換"),
    ("Select\nObjects", "オブジェクトの選択"),
    ("Themes", "テーマ"),
    ("Margins", "余白"),
    ("Left Margin", "左余白"),
    ("Right Margin", "右余白"),
    ("Top Margin", "上余白"),
    ("Bottom Margin", "下余白"),
    ("Header Margin", "ヘッダーの余白"),
    ("Footer Margin", "フッターの余白"),
    ("Size", "サイズ"),
    ("Print\nArea", "印刷範囲"),
    ("Set Print Area", "印刷範囲の設定"),
    ("Breaks", "改ページ"),
    ("Insert Page Break", "改ページの挿入"),
    ("Remove Page Break", "改ページの解除"),
    ("Reset All Page Breaks", "すべての改ページを解除"),
    ("Print\nTitles", "印刷タイトル"),
    ("Sheet\nRight-to-Left", "シートを右から左へ"),
    ("Sheet Right-to-Left", "シートを右から左へ"),
    ("Narrow", "狭い"),
    ("Wide", "広い"),
    ("Normal", "標準"),
    ("Portrait", "縦"),
    ("Landscape", "横"),
    ("Custom Margins…", "ユーザー設定の余白…"),
    ("Insert\nFunction", "関数の挿入"),
    ("Name\nManager", "名前の管理"),
    ("Define Name", "名前の定義"),
    ("Use in Formula", "数式で使用"),
    ("Create from Selection", "選択範囲から作成"),
    ("Trace Precedents", "参照元のトレース"),
    ("Trace Dependents", "参照先のトレース"),
    ("Remove Arrows", "矢印の解除"),
    ("Show Formulas", "数式の表示"),
    ("Error Checking", "エラーチェック"),
    ("Evaluate Formula", "数式の検証"),
    ("Watch Window", "ウォッチ ウィンドウ"),
    ("Calculation\nOptions", "計算方法の設定"),
    ("Calculate Now", "再計算実行"),
    ("Calculate Sheet", "シートの再計算"),
    ("Automatic", "自動"),
    ("Automatic Except for Data Tables", "データ テーブル以外は自動"),
    ("Manual", "手動"),
    ("Sort &\nFilter", "並べ替えとフィルター"),
    ("Sort", "並べ替え"),
    ("Sort A to Z", "昇順"),
    ("Sort Z to A", "降順"),
    ("Custom Sort…", "ユーザー設定の並べ替え…"),
    ("Filter", "フィルター"),
    ("Reapply", "再適用"),
    ("Get Data\n(Text/CSV)", "データの取得 (テキスト/CSV)"),
    ("Remove\nDuplicates", "重複の削除"),
    ("Data\nValidation", "データの入力規則"),
    ("Data Validation", "データの入力規則"),
    ("Data Validation…", "データの入力規則…"),
    ("Circle Invalid Data", "無効なデータをマーク"),
    ("Clear Validation", "入力規則のクリア"),
    ("Group", "グループ化"),
    ("Ungroup", "グループ解除"),
    ("Show Detail", "詳細データの表示"),
    ("Hide Detail", "詳細を表示しない"),
    ("Subtotal", "小計"),
    ("Data\nAnalysis", "データ分析"),
    ("Data Analysis", "データ分析"),
    ("Solver", "ソルバー"),
    ("What-If\nAnalysis", "What-If 分析"),
    ("Data Table…", "データ テーブル…"),
    ("Scenario Manager…", "シナリオ マネージャー…"),
    ("Goal Seek…", "ゴール シーク…"),
    ("Forecast\nSheet", "予測シート"),
    ("Spelling", "スペルチェック"),
    ("Check\nAccessibility", "アクセシビリティ チェック"),
    ("Threaded Comments", "スレッド化されたコメント"),
    ("Show\nComments", "コメントの表示"),
    ("Previous", "前へ"),
    ("Next", "次へ"),
    ("Previous Comment", "前のコメント"),
    ("Next Comment", "次のコメント"),
    ("Show All Notes", "すべてのメモを表示"),
    ("Convert to Comments", "コメントに変換"),
    ("Protect\nWorkbook", "ブックの保護"),
    ("Allow Edit\nRanges", "範囲の編集を許可"),
    ("Allow Edit Ranges", "範囲の編集を許可"),
    ("Unprotect\nSheet", "シート保護の解除"),
    ("Protect Sheet…", "シートの保護…"),
    ("Comments", "コメント"),
    ("Notes", "メモ"),
    ("Page Break\nPreview", "改ページ プレビュー"),
    ("Page\nLayout", "ページ レイアウト"),
    ("Formula Bar", "数式バー"),
    ("Gridlines", "目盛線"),
    ("Headings", "見出し"),
    ("Zoom", "ズーム"),
    ("100%", "100%"),
    ("Zoom to\nSelection", "選択範囲を拡大"),
    ("Freeze\nPanes", "ウィンドウ枠の固定"),
    ("Freeze Panes", "ウィンドウ枠の固定"),
    ("Freeze Top Row", "最上行の固定"),
    ("Freeze First Column", "最左列の固定"),
    ("Unfreeze Panes", "ウィンドウ枠固定の解除"),
    ("Dark Mode", "ダーク モード"),
    ("Display theme", "表示テーマ"),
    ("Animations", "アニメーション"),
    (
        "Glide the selection outline to the cell you move to instead of jumping there",
        "選択範囲の枠を、移動先のセルまで滑らかに動かします（瞬時には切り替えません）",
    ),
    ("System", "システム"),
    ("Light", "ライト"),
    ("Dark", "ダーク"),
    ("Interface language", "表示言語"),
    ("Interface size", "表示サイズ"),
    ("Auto", "自動"),
    // Background jobs (docs/plano-responsividade.md): the status bar indicators.
    ("Saving…", "保存中…"),
    ("Calculating…", "計算中…"),
    ("Drag to freeze columns", "ドラッグして列を固定"),
    ("Drag to freeze rows", "ドラッグして行を固定"),
    ("Command\nPalette", "コマンド パレット"),
    ("Agent\nControl", "エージェント制御"),
    ("Action\nJournal", "操作ジャーナル"),
    ("About\nGridCraft", "GridCraft について"),
    ("Share", "共有"),
    ("Add Chart\nElement", "グラフ要素を追加"),
    ("Change\nChart Type", "グラフの種類の変更"),
    ("Switch\nRow/Column", "行/列の切り替え"),
    ("Select\nData", "データの選択"),
    ("Data Labels: None", "データ ラベル: なし"),
    ("Data Labels: Show", "データ ラベル: 表示"),
    ("Legend: None", "凡例: なし"),
    ("Legend: Bottom", "凡例: 下"),
    ("Legend: Top", "凡例: 上"),
    ("Legend: Right", "凡例: 右"),
    ("Gridlines: None", "目盛線: なし"),
    ("Gridlines: Show", "目盛線: 表示"),
    ("Chart\nTitle", "グラフ タイトル"),
    ("Delete\nChart", "グラフの削除"),
    ("Convert\nto Range", "範囲に変換"),
    ("Table\nStyles", "テーブル スタイル"),
    ("Top 10 Items", "上位 10 項目"),
    ("Top 10%", "上位 10%"),
    ("Bottom 10 Items", "下位 10 項目"),
    ("Bottom 10%", "下位 10%"),
    ("Above Average", "平均より上"),
    ("Below Average", "平均より下"),
    ("Clear Rules from Entire Sheet", "シート全体からルールをクリア"),
    ("Clear Rules from Selected Cells", "選択したセルからルールをクリア"),
    ("Constants", "定数"),
    ("Data Bars", "データ バー"),
    ("Color Scales", "カラー スケール"),
    ("Icon Sets", "アイコン セット"),
    ("Home", "ホーム"),
    ("Draw", "描画"),
    ("Page Layout", "ページ レイアウト"),
    ("Formulas", "数式"),
    ("Data", "データ"),
    ("Review", "レビュー"),
    ("View", "表示"),
    ("Automate", "自動化"),
    ("Table Design", "テーブル デザイン"),
    ("Chart Design", "グラフ デザイン"),
    ("Ready", "準備完了"),
    ("Enter", "入力"),
    ("Edit", "編集"),
    ("Point", "ポイント"),
    ("Calculate", "計算"),
    ("Select destination and press Enter or choose Paste", "貼り付け先を選び、Enter キーを押すか [貼り付け] を選びます"),
    ("General", "標準"),
    ("Number", "数値"),
    ("Currency", "通貨"),
    ("Accounting", "会計"),
    ("Date", "日付"),
    ("Time", "時刻"),
    ("Percentage", "パーセンテージ"),
    ("Fraction", "分数"),
    ("Scientific", "指数"),
    ("Text", "文字列"),
    ("Special", "特殊"),
    ("Custom", "ユーザー設定"),
    ("Short Date", "日付"),
    ("Long Date", "長い日付"),
    ("More Number Formats…", "その他の表示形式…"),
    ("Financial", "財務"),
    ("Logical", "論理"),
    ("Date &\nTime", "日付/時刻"),
    ("Lookup &\nReference", "検索/行列"),
    ("Math &\nTrig", "数学/三角"),
    ("More\nFunctions", "その他の関数"),
    ("Sheet Options|View", "画面"),
    ("Print", "印刷"),
    // Page Layout view and the Header & Footer tab.
    ("Click to add data", "クリックしてデータを追加"),
    ("Add header", "ヘッダーの追加"),
    ("Add footer", "フッターの追加"),
    ("(none)", "(なし)"),
    ("Header", "ヘッダー"),
    ("Footer", "フッター"),
    ("Page\nNumber", "ページ\n番号"),
    ("Number\nof Pages", "ページ数"),
    ("Current\nDate", "現在の\n日付"),
    ("Current\nTime", "現在の\n時刻"),
    ("File\nPath", "ファイルの\nパス"),
    ("File\nName", "ファイル名"),
    ("Sheet\nName", "シート名"),
    ("Go to\nHeader", "ヘッダーに\n移動"),
    ("Go to\nFooter", "フッターに\n移動"),
    ("Page Number", "ページ番号"),
    ("Number of Pages", "ページ数"),
    ("Current Date", "現在の日付"),
    ("Current Time", "現在の時刻"),
    ("File Path", "ファイルのパス"),
    ("File Name", "ファイル名"),
    ("Sheet Name", "シート名"),
    ("Go to Header", "ヘッダーに移動"),
    ("Different First Page", "先頭ページのみ別指定"),
    ("Different Odd & Even Pages", "奇数/偶数ページ別指定"),
    ("Go to Footer", "フッターに移動"),
    ("Ruler", "ルーラー"),
    // File › Backstage view.
    ("File", "ファイル"),
    ("Back", "戻る"),
    ("New", "新規"),
    ("Open", "開く"),
    ("Info", "情報"),
    ("Save", "上書き保存"),
    ("Save As", "名前を付けて保存"),
    ("Export", "エクスポート"),
    ("Close", "閉じる"),
    ("Options", "オプション"),
    ("Blank workbook", "空白のブック"),
    ("Recent", "最近使ったアイテム"),
    ("No recent workbooks yet.", "最近使ったブックはまだありません。"),
    ("More workbooks", "その他のブック"),
    ("Modified", "更新日時"),
    ("Not available", "利用できません"),
    ("Just now", "たった今"),
    ("Yesterday", "昨日"),
    ("{n} min ago", "{n} 分前"),
    ("{n} h ago", "{n} 時間前"),
    ("{n} days ago", "{n} 日前"),
    ("{n} weeks ago", "{n} 週間前"),
    ("{n} months ago", "{n} か月前"),
    ("{n} years ago", "{n} 年前"),
    ("Browse…", "参照…"),
    ("Not saved yet", "まだ保存されていません"),
    ("Location", "場所"),
    ("Sheets", "シート"),
    ("Protect Workbook", "ブックの保護"),
    (
        "Sheet structure is locked: sheets can't be added, moved, renamed or deleted.",
        "シートの構成はロックされています。シートの追加、移動、名前の変更、削除はできません。",
    ),
    (
        "Lock the sheet structure so sheets can't be added, moved, renamed or deleted.",
        "シートの構成をロックして、シートの追加、移動、名前の変更、削除をできないようにします。",
    ),
    ("Properties", "プロパティ"),
    ("Title", "タイトル"),
    ("Subject", "件名"),
    ("Author", "作成者"),
    ("Company", "会社名"),
    ("Keywords", "キーワード"),
    ("Description", "説明"),
    ("No workbook is open.", "ブックが開かれていません。"),
    ("Save as type", "ファイルの種類"),
    ("Excel Workbook (*.xlsx)", "Excel ブック (*.xlsx)"),
    ("CSV UTF-8 (Comma delimited) (*.csv)", "CSV UTF-8 (コンマ区切り) (*.csv)"),
    ("Web Page (*.html)", "Web ページ (*.html)"),
    ("PDF (*.pdf)", "PDF ファイル (*.pdf)"),
    ("Download", "ダウンロード"),
    (
        "A CSV file, web page or PDF is a copy: the workbook keeps its own file.",
        "CSV ファイル、Web ページ、PDF はコピーです。ブックは元のファイルのままです。",
    ),
    ("Save the workbook under another name or in another folder.", "ブックを別の名前または別のフォルダーに保存します。"),
    ("Print Active Sheets", "作業中のシートを印刷"),
    ("Print Entire Workbook", "ブック全体を印刷"),
    ("Fit Sheet on One Page", "シートを 1 ページに印刷"),
    ("Page Setup…", "ページ設定…"),
    ("Page {page} of {pages}", "{page} / {pages} ページ"),
    ("Nothing to print.", "印刷する内容がありません。"),
    ("Previous page", "前のページ"),
    ("Next page", "次のページ"),
    ("Create PDF", "PDF の作成"),
    ("Save the sheet as a PDF document that keeps its page layout.", "ページ レイアウトを保ったまま、シートを PDF ドキュメントとして保存します。"),
    ("Change File Type", "ファイルの種類の変更"),
    ("CSV (Comma delimited)", "CSV (コンマ区切り)"),
    ("Save the active sheet as comma-separated text.", "作業中のシートをコンマ区切りのテキストとして保存します。"),
    ("Web Page (.html)", "Web ページ (.html)"),
    ("Save the workbook as a web page.", "ブックを Web ページとして保存します。"),
    ("Options|General", "全般"),
    ("Options|Save", "保存"),
    ("Show formula bar", "数式バーを表示する"),
    ("Enable AutoComplete for cell values", "セルの値にオートコンプリートを使用する"),
    ("Save .xlsx workbooks automatically after every change", "変更のたびに .xlsx ブックを自動的に保存する"),
    ("Workbook calculation", "ブックの計算"),
    ("Enable iterative calculation", "反復計算を行う"),
    ("Calculation settings are saved with the workbook.", "計算の設定はブックと共に保存されます。"),
];

/// `name` lowercased with the Latin accents that occur in language names folded away, so a
/// byte-wise sort reads alphabetically. Other scripts are left as they are.
fn name_sort_key(name: &str) -> String {
    name.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à'..='å' | 'ā' | 'ă' | 'ą' => 'a',
            'ç' | 'ć' | 'č' => 'c',
            'è'..='ë' | 'ē' | 'ě' | 'ę' | 'ế' | 'ệ' => 'e',
            'ì'..='ï' | 'ī' => 'i',
            'ñ' | 'ń' | 'ň' => 'n',
            'ò'..='ö' | 'ø' | 'ő' => 'o',
            'ś' | 'š' => 's',
            'ù'..='ü' | 'ū' | 'ů' | 'ű' => 'u',
            'ý' | 'ÿ' => 'y',
            'ź' | 'ż' | 'ž' => 'z',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Help tables are sorted and unique: lookups binary-search them.
    #[test]
    fn function_help_tables_are_sorted_and_unique() {
        for language in Language::ALL {
            let Some((description_table, argument_table)) = language.function_help() else { continue };
            for table in [description_table, argument_table] {
                assert!(table.windows(2).all(|pair| pair[0].0 < pair[1].0), "{language:?} function help is not sorted and unique");
                for (english, translated) in table {
                    assert!(!translated.is_empty(), "{language:?}: empty translation of {english:?}");
                }
            }
        }
    }

    #[test]
    fn untranslated_text_falls_back_to_english() {
        let label = |key: &'static str| key.rsplit_once('|').map_or(key, |(_, label)| label);
        for (english, _) in ja::CORE {
            assert_eq!(Language::En.tr(english), label(english));
        }
        for language in Language::ALL {
            assert_eq!(language.tr("Sheet1!A1"), "Sheet1!A1");
            assert_eq!(language.tr("Not in any catalog|Untranslated"), "Untranslated");
        }
        // A `context|label` key shows only its label in English.
        assert_eq!(Language::En.tr("Page Layout|Orientation"), "Orientation");
        // Errors drop the `command `id`:` prefix before translating; an untranslated one stays whole.
        assert_eq!(Language::PtBr.tr_error("command `chart`: Select a chart first."), Language::PtBr.tr("Select a chart first."));
        assert_eq!(Language::PtBr.tr_error("Sheet|Unknown failure"), "Sheet|Unknown failure");
        assert_eq!(Language::PtBr.tr_error("Printer Office: permission denied"), "Printer Office: permission denied");
        assert_eq!(Language::parse("xx"), None);
    }

    /// A catalog whose translations slip onto their neighbours' keys still covers every key, so
    /// check the shape of each translation against its English: placeholders survive, and a
    /// one-line label (a menu item, a tooltip) doesn't turn into a two-line ribbon label.
    #[test]
    fn translations_keep_the_shape_of_their_english() {
        let placeholders =
            |s: &str| s.split('{').skip(1).filter_map(|rest| rest.split_once('}').map(|(name, _)| name.to_string())).collect::<Vec<_>>();
        for language in Language::ALL {
            for (english, _) in ja::CORE {
                let label = english.rsplit_once('|').map_or(*english, |(_, label)| label);
                let text = language.tr(english);
                assert_eq!(placeholders(text), placeholders(label), "{language:?}: {english:?} => {text:?}");
                // Big ribbon buttons whose English fits on one line may still need two.
                if !label.contains('\n') && !["AutoSum", "PivotTable"].contains(english) {
                    assert!(!text.contains('\n'), "{language:?}: one-line {english:?} => {text:?}");
                }
            }
        }
    }

    /// The screen-reader names for the formula-bar controls must be translated in every catalog
    /// (no silent English fallback) and stay single-line labels.
    #[test]
    fn accessible_control_names_are_translated_in_every_catalog() {
        for key in ["Name Box", "Cell Editor", "Font Size", "Sheet Name", "Text Box"] {
            for language in Language::ALL {
                let shown = language.tr(key);
                assert!(!shown.contains('\n'), "{}: {key:?} is not a single-line label: {shown:?}", language.code());
                if language != Language::En {
                    assert_ne!(shown, key, "{} falls back to English for {key:?}", language.code());
                }
            }
        }
    }

    #[test]
    fn translations_never_show_the_context_prefix() {
        // `tr` returns a found translation as it is, so a catalog that keeps `context|` in its
        // value shows it on screen (File › Options read "Options|Général" in French).
        let keys = ja::CORE.iter().map(|(english, _)| *english).chain(["Options|General", "Options|Save", "Sheet Options|View"]);
        for english in keys {
            for language in Language::ALL {
                let shown = language.tr(english);
                assert!(!shown.contains('|'), "{} shows a context prefix for {english}: {shown}", language.code());
            }
        }
    }

    #[test]
    fn unknown_saved_language_keeps_the_other_preferences() {
        let ui: crate::UiState = serde_json::from_str(r#"{"dark": true, "language": "xx"}"#).unwrap();
        assert!(ui.dark);
        assert_eq!(ui.language, Language::En);
        let ui: crate::UiState = serde_json::from_str(r#"{"dark": true, "language": 7}"#).unwrap();
        assert!(ui.dark);
        let ui: crate::UiState = serde_json::from_str(r#"{"language": "ja"}"#).unwrap();
        assert_eq!(ui.language, Language::Ja);
        assert_eq!(crate::UiState::default().language, Language::En, "the default never reads the host locale");
        assert_eq!(saved_language(&serde_json::json!({"language": "ja"})), Some(Language::Ja));
        assert_eq!(saved_language(&serde_json::json!({"language": "xx"})), None);
        assert_eq!(saved_language(&serde_json::json!({})), None);
    }

    #[test]
    fn tags_reduce_to_supported_languages() {
        assert_eq!(Language::from_tag("ja-JP"), Some(Language::Ja));
        assert_eq!(Language::from_tag("en_US"), Some(Language::En));
        assert_eq!(Language::from_tag("es_MX"), Some(Language::Es));
        assert_eq!(Language::from_tag("el-GR"), Some(Language::El));
        assert_eq!(Language::from_tag("el_GR.UTF-8"), Some(Language::El));
        assert_eq!(Language::parse("el"), Some(Language::El));
        assert_eq!(Language::from_tag("zh-Hans-CN"), Some(Language::Zh));
        assert_eq!(Language::from_tag("ko-KR"), Some(Language::Ko));
        assert_eq!(Language::from_tag("RU_ru"), Some(Language::Ru));
        assert_eq!(Language::from_tag("de-DE"), Some(Language::De));
        assert_eq!(Language::from_tag("de_AT"), Some(Language::De));
    }

    #[test]
    fn tags_reduce_to_supported_languages_with_locale_suffixes() {
        assert_eq!(Language::from_tag("de-CH"), Some(Language::De));
        assert_eq!(Language::from_tag("pt-BR"), Some(Language::PtBr));
        assert_eq!(Language::parse("pt"), Some(Language::PtBr));
        assert_eq!(Language::parse("pt-br"), Some(Language::PtBr));
        assert_eq!(Language::from_tag("cs-CZ"), Some(Language::Cs));
        assert_eq!(Language::from_tag("cs_CZ.UTF-8"), Some(Language::Cs));
        assert_eq!(Language::parse("cs"), Some(Language::Cs));
        assert_eq!(Language::from_tag("fr-FR"), Some(Language::Fr));
        assert_eq!(Language::from_tag("fr_CA"), Some(Language::Fr));
        assert_eq!(Language::parse("fr"), Some(Language::Fr));
        assert_eq!(Language::from_tag("sr-Latn-RS"), Some(Language::Sr));
    }

    #[test]
    fn tags_reduce_to_other_supported_languages() {
        assert_eq!(Language::from_tag("sr-RS"), Some(Language::Sr));
        assert_eq!(Language::parse("sr"), Some(Language::Sr));
        assert_eq!(Language::from_tag("tr-TR"), Some(Language::Tr));
        assert_eq!(Language::from_tag("tr_TR.UTF-8"), Some(Language::Tr));
        assert_eq!(Language::parse("tr"), Some(Language::Tr));
        assert_eq!(Language::from_tag("pl-PL"), Some(Language::Pl));
        assert_eq!(Language::from_tag("pl_PL.UTF-8"), Some(Language::Pl));
        assert_eq!(Language::parse("pl"), Some(Language::Pl));
        assert_eq!(Language::from_tag("it-IT"), Some(Language::It));
        assert_eq!(Language::from_tag("it_CH.UTF-8"), Some(Language::It));
        assert_eq!(Language::parse("it"), Some(Language::It));
    }

    #[test]
    fn tags_reduce_to_remaining_supported_languages() {
        assert_eq!(Language::from_tag("uk-XX"), Some(Language::Uk));
        assert_eq!(Language::parse("uk"), Some(Language::Uk));
        assert_eq!(Language::from_tag("vi-VN"), Some(Language::Vi));
        assert_eq!(Language::parse("vi"), Some(Language::Vi));
        assert_eq!(Language::from_tag("nl-NL"), Some(Language::Nl));
        assert_eq!(Language::from_tag("nl_BE"), Some(Language::Nl));
        assert_eq!(Language::parse("nl"), Some(Language::Nl));
        assert_eq!(Language::from_tag("ro-RO"), Some(Language::Ro));
        assert_eq!(Language::from_tag("ro_MD.UTF-8"), Some(Language::Ro));
        assert_eq!(Language::parse("ro"), Some(Language::Ro));
    }
    #[test]
    fn an_unset_language_preference_is_english_until_the_app_applies_the_system_one() {
        // Deserializing never reads the host locale (tests stay deterministic); the desktop app
        // applies `Language::system()` itself when `ui.json` names no usable language.
        let restored: crate::UiState = serde_json::from_str("{}").unwrap();
        assert_eq!(restored.language, Language::En);
        assert_eq!(saved_language(&serde_json::json!({"language": "zh"})), Some(Language::Zh));
    }

    #[test]
    fn language_commands_switch_and_persist() {
        let mut app = crate::SheetApp::new(gridcraft_engine::Session::default(), Default::default());
        for language in Language::ALL {
            app.run("app.language.set", serde_json::json!({"language": language.code()})).unwrap();
            assert_eq!(app.ui.language, language);
            let restored: crate::UiState = serde_json::from_str(&serde_json::to_string(&app.ui).unwrap()).unwrap();
            assert_eq!(restored.language, language);
        }
        app.run("app.language.set", serde_json::json!({"code": "ja"})).unwrap();
        assert_eq!(app.ui.language, Language::Ja, "`code` is an alias for `language`");
        assert!(app.run("app.language.set", serde_json::json!({})).is_err(), "no language is an error, not a dialog");
        let unknown = app.run("app.language.set", serde_json::json!({"language": "xx"})).unwrap_err().to_string();
        for l in Language::ALL {
            assert!(unknown.contains(&format!("\"{}\"", l.code())), "the error lists {:?}: {unknown}", l.code());
        }
        app.run("app.language.english", serde_json::json!({})).unwrap();
        assert_eq!(app.ui.language, Language::En);
        for l in Language::ALL {
            assert_eq!(Language::parse(l.code()), Some(l));
        }
    }

    #[test]
    fn picker_lists_english_first_then_names_alphabetically() {
        let order = Language::for_picker();
        assert_eq!(order.first(), Some(&Language::En));
        assert_eq!(order.len(), Language::ALL.len());
        let keys: Vec<String> = order[1..].iter().map(|l| name_sort_key(l.name())).collect();
        assert!(keys.windows(2).all(|w| w[0] <= w[1]), "{keys:?}");
        assert_eq!(name_sort_key("Čeština"), "cestina");
    }
}
