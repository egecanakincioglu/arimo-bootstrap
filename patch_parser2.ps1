
Set-ExecutionPolicy -ExecutionPolicy Bypass -Scope Process -Force

$file = "C:\Users\Arimo\Desktop\arimo-compiler\.claude\worktrees\admiring-gauss-518635\src\parser\mod.rs"
$content = [System.IO.File]::ReadAllText($file, [System.Text.Encoding]::UTF8)

# =============================================================================
# 1. parse_class annotation loop: add async_ variable + annotation + keyword
# =============================================================================

# In parse_class: add "let mut async_ = false;" after "let mut inline_ = false;"
# Also add "async" => { async_ = true; } to the annotation match
# Also add "let async_ = async_ || self.eat(&Token::Async);" after the static_ line

$old1 = "        while !self.check(&Token::RBrace) && !self.check(&Token::Eof) {`r`n            let mut inline_ = false;`r`n            let mut calling_conv: Option<CallingConv> = None;`r`n            let mut section: Option<String> = None;`r`n            while self.check(&Token::At) {`r`n                self.advance();`r`n                let ann = self.expect_ident()?;`r`n                match ann.as_str() {`r`n                    `"inline`"    => { inline_ = true; }`r`n                    `"cdecl`"     => { calling_conv = Some(CallingConv::Cdecl); }`r`n                    `"stdcall`"   => { calling_conv = Some(CallingConv::Stdcall); }`r`n                    `"interrupt`" => { calling_conv = Some(CallingConv::Interrupt); }`r`n                    `"section`"   => {`r`n                        self.expect(&Token::LParen)?;`r`n                        match self.current().clone() {`r`n                            Token::Str(s) => { section = Some(s); self.advance(); }`r`n                            _ => {`r`n                                let (line, col) = self.current_span();`r`n                                return Err(ParseError::new(`"@section expects a string literal`", line, col));`r`n                            }`r`n                        }`r`n                        self.expect(&Token::RParen)?;`r`n                    }`r`n                    other => {`r`n                        let (line, col) = self.current_span();`r`n                        return Err(ParseError::new(`r`n                            &format!(`"unknown method annotation '@{}'`", other),`r`n                            line, col,`r`n                        ));`r`n                    }`r`n                }`r`n            }`r`n`r`n            let vis       = self.parse_visibility()?;`r`n            let static_   = self.eat(&Token::Static);`r`n            let readonly  = self.eat(&Token::Readonly);`r`n            let abstract_ = self.eat(&Token::Abstract);`r`n            let override_ = self.eat(&Token::Override);"

$new1 = "        while !self.check(&Token::RBrace) && !self.check(&Token::Eof) {`r`n            let mut inline_ = false;`r`n            let mut async_  = false;`r`n            let mut calling_conv: Option<CallingConv> = None;`r`n            let mut section: Option<String> = None;`r`n            while self.check(&Token::At) {`r`n                self.advance();`r`n                let ann = self.expect_ident()?;`r`n                match ann.as_str() {`r`n                    `"inline`"    => { inline_ = true; }`r`n                    `"async`"     => { async_ = true; }`r`n                    `"cdecl`"     => { calling_conv = Some(CallingConv::Cdecl); }`r`n                    `"stdcall`"   => { calling_conv = Some(CallingConv::Stdcall); }`r`n                    `"interrupt`" => { calling_conv = Some(CallingConv::Interrupt); }`r`n                    `"section`"   => {`r`n                        self.expect(&Token::LParen)?;`r`n                        match self.current().clone() {`r`n                            Token::Str(s) => { section = Some(s); self.advance(); }`r`n                            _ => {`r`n                                let (line, col) = self.current_span();`r`n                                return Err(ParseError::new(`"@section expects a string literal`", line, col));`r`n                            }`r`n                        }`r`n                        self.expect(&Token::RParen)?;`r`n                    }`r`n                    other => {`r`n                        let (line, col) = self.current_span();`r`n                        return Err(ParseError::new(`r`n                            &format!(`"unknown method annotation '@{}'`", other),`r`n                            line, col,`r`n                        ));`r`n                    }`r`n                }`r`n            }`r`n`r`n            let vis       = self.parse_visibility()?;`r`n            let static_   = self.eat(&Token::Static);`r`n            let async_    = async_ || self.eat(&Token::Async);`r`n            let readonly  = self.eat(&Token::Readonly);`r`n            let abstract_ = self.eat(&Token::Abstract);`r`n            let override_ = self.eat(&Token::Override);"

if ($content.Contains($old1)) {
    $content = $content.Replace($old1, $new1)
    Write-Host "parse_class annotation loop: UPDATED"
} else {
    Write-Host "parse_class annotation loop: NOT FOUND"
}

# =============================================================================
# 2. parse_struct annotation loop: add async_ variable + annotation + keyword
# =============================================================================

$old2 = "        while !self.check(&Token::RBrace) && !self.check(&Token::Eof) {`r`n            let mut inline_ = false;`r`n            let mut calling_conv: Option<CallingConv> = None;`r`n            let mut section: Option<String> = None;`r`n            while self.check(&Token::At) {`r`n                self.advance();`r`n                let ann = self.expect_ident()?;`r`n                match ann.as_str() {`r`n                    `"inline`"    => { inline_ = true; }`r`n                    `"cdecl`"     => { calling_conv = Some(CallingConv::Cdecl); }`r`n                    `"stdcall`"   => { calling_conv = Some(CallingConv::Stdcall); }`r`n                    `"interrupt`" => { calling_conv = Some(CallingConv::Interrupt); }`r`n                    `"section`"   => {`r`n                        self.expect(&Token::LParen)?;`r`n                        match self.current().clone() {`r`n                            Token::Str(s) => { section = Some(s); self.advance(); }`r`n                            _ => {`r`n                                let (line, col) = self.current_span();`r`n                                return Err(ParseError::new(`"@section expects a string literal`", line, col));`r`n                            }`r`n                        }`r`n                        self.expect(&Token::RParen)?;`r`n                    }`r`n                    other => {`r`n                        let (line, col) = self.current_span();`r`n                        return Err(ParseError::new(`r`n                            &format!(`"unknown method annotation '@{}'`", other),`r`n                            line, col,`r`n                        ));`r`n                    }`r`n                }`r`n            }`r`n`r`n            let vis       = self.parse_visibility()?;`r`n            let static_   = self.eat(&Token::Static);`r`n            let _readonly = self.eat(&Token::Readonly); // struct fields are value-copied, readonly is on field`r`n            let override_ = self.eat(&Token::Override);"

$new2 = "        while !self.check(&Token::RBrace) && !self.check(&Token::Eof) {`r`n            let mut inline_ = false;`r`n            let mut async_  = false;`r`n            let mut calling_conv: Option<CallingConv> = None;`r`n            let mut section: Option<String> = None;`r`n            while self.check(&Token::At) {`r`n                self.advance();`r`n                let ann = self.expect_ident()?;`r`n                match ann.as_str() {`r`n                    `"inline`"    => { inline_ = true; }`r`n                    `"async`"     => { async_ = true; }`r`n                    `"cdecl`"     => { calling_conv = Some(CallingConv::Cdecl); }`r`n                    `"stdcall`"   => { calling_conv = Some(CallingConv::Stdcall); }`r`n                    `"interrupt`" => { calling_conv = Some(CallingConv::Interrupt); }`r`n                    `"section`"   => {`r`n                        self.expect(&Token::LParen)?;`r`n                        match self.current().clone() {`r`n                            Token::Str(s) => { section = Some(s); self.advance(); }`r`n                            _ => {`r`n                                let (line, col) = self.current_span();`r`n                                return Err(ParseError::new(`"@section expects a string literal`", line, col));`r`n                            }`r`n                        }`r`n                        self.expect(&Token::RParen)?;`r`n                    }`r`n                    other => {`r`n                        let (line, col) = self.current_span();`r`n                        return Err(ParseError::new(`r`n                            &format!(`"unknown method annotation '@{}'`", other),`r`n                            line, col,`r`n                        ));`r`n                    }`r`n                }`r`n            }`r`n`r`n            let vis       = self.parse_visibility()?;`r`n            let static_   = self.eat(&Token::Static);`r`n            let async_    = async_ || self.eat(&Token::Async);`r`n            let _readonly = self.eat(&Token::Readonly); // struct fields are value-copied, readonly is on field`r`n            let override_ = self.eat(&Token::Override);"

if ($content.Contains($old2)) {
    $content = $content.Replace($old2, $new2)
    Write-Host "parse_struct annotation loop: UPDATED"
} else {
    Write-Host "parse_struct annotation loop: NOT FOUND"
}

# =============================================================================
# 3. Update all parse_method_body call sites — add false, false for default_ async_
# =============================================================================

# parse_class: operator overloading call (has abstract_)
$old3 = "parse_method_body(vis, static_, abstract_, override_, inline_, calling_conv, section, method_name, None)?"
$new3 = "parse_method_body(vis, static_, abstract_, false, override_, inline_, async_, calling_conv, section, method_name, None)?"
$count3 = ($content.Split($old3).Count - 1)
$content = $content.Replace($old3, $new3)
Write-Host "parse_class operator overload call: $count3 replaced"

# parse_class: method call with iname (has abstract_) — multiline
$old4 = "parse_method_body(`r`n                                    vis, static_, abstract_, override_, inline_, calling_conv, section, iname, None`r`n                                )?"
$new4 = "parse_method_body(`r`n                                    vis, static_, abstract_, false, override_, inline_, async_, calling_conv, section, iname, None`r`n                                )?"
$count4 = ($content.Split($old4).Count - 1)
$content = $content.Replace($old4, $new4)
Write-Host "parse_class method iname call: $count4 replaced"

# parse_class: method call with name+type (has abstract_) — multiline
$old5 = "parse_method_body(`r`n                                vis, static_, abstract_, override_, inline_, calling_conv, section, name, Some(ty)`r`n                            )?"
$new5 = "parse_method_body(`r`n                                vis, static_, abstract_, false, override_, inline_, async_, calling_conv, section, name, Some(ty)`r`n                            )?"
$count5 = ($content.Split($old5).Count - 1)
$content = $content.Replace($old5, $new5)
Write-Host "parse_class method name+type call: $count5 replaced"

# parse_struct: operator overload call (has false for abstract_)
$old6 = "parse_method_body(vis, static_, false, override_, inline_, calling_conv, section, method_name, None)?"
$new6 = "parse_method_body(vis, static_, false, false, override_, inline_, async_, calling_conv, section, method_name, None)?"
$count6 = ($content.Split($old6).Count - 1)
$content = $content.Replace($old6, $new6)
Write-Host "parse_struct operator overload call: $count6 replaced"

# parse_struct: method call with iname — multiline
$old7 = "parse_method_body(`r`n                                    vis, static_, false, override_, inline_, calling_conv, section, iname, None`r`n                                )?"
$new7 = "parse_method_body(`r`n                                    vis, static_, false, false, override_, inline_, async_, calling_conv, section, iname, None`r`n                                )?"
$count7 = ($content.Split($old7).Count - 1)
$content = $content.Replace($old7, $new7)
Write-Host "parse_struct method iname call: $count7 replaced"

# parse_struct: method call with name+type — multiline
$old8 = "parse_method_body(`r`n                                vis, static_, false, override_, inline_, calling_conv, section, name, Some(ty)`r`n                            )?"
$new8 = "parse_method_body(`r`n                                vis, static_, false, false, override_, inline_, async_, calling_conv, section, name, Some(ty)`r`n                            )?"
$count8 = ($content.Split($old8).Count - 1)
$content = $content.Replace($old8, $new8)
Write-Host "parse_struct method name+type call: $count8 replaced"

# parse_enum: method call (no inline, no async_)
$old9 = "self.parse_method_body(vis, static_, false, false, false, None, None, name, None)?"
$new9 = "self.parse_method_body(vis, static_, false, false, false, false, false, None, None, name, None)?"
$count9 = ($content.Split($old9).Count - 1)
$content = $content.Replace($old9, $new9)
Write-Host "parse_enum method call: $count9 replaced"

[System.IO.File]::WriteAllText($file, $content, [System.Text.Encoding]::UTF8)
Write-Host "File written."
