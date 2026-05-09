
$file = "C:\Users\Arimo\Desktop\arimo-compiler\.claude\worktrees\admiring-gauss-518635\src\parser\mod.rs"
$content = [System.IO.File]::ReadAllText($file, [System.Text.Encoding]::UTF8)

# 1. Update parse_if — add @likely/@unlikely hint parsing
$old_if_end = "        Ok(Stmt::If { cond, then, else_if, else_ })`r`n    }"
$new_if_end = "        Ok(Stmt::If { hint, cond, then, else_if, else_ })`r`n    }"

# Insert hint parsing after "self.expect(&Token::If)?"
$old_if_start = "    fn parse_if(&mut self) -> ParseResult<Stmt> {`r`n        self.expect(&Token::If)?;`r`n        self.expect(&Token::LParen)?;"
$new_if_start = @"
    fn parse_if(&mut self) -> ParseResult<Stmt> {
        self.expect(&Token::If)?;
        // @likely / @unlikely hint — optional annotation
        let hint = if self.check(&Token::At) {
            self.advance();
            let ann = self.expect_ident()?;
            match ann.as_str() {
                "likely"   => Some(BranchHint::Likely),
                "unlikely" => Some(BranchHint::Unlikely),
                other => {
                    let (line, col) = self.current_span();
                    return Err(ParseError::new(
                        &format!("unknown if annotation '@{}' — only @likely and @unlikely are supported", other),
                        line, col,
                    ));
                }
            }
        } else {
            None
        };
        self.expect(&Token::LParen)?;
"@
# Convert to CRLF
$new_if_start = $new_if_start -replace "`n", "`r`n"

if ($content.Contains($old_if_start)) {
    $content = $content.Replace($old_if_start, $new_if_start)
    Write-Host "parse_if start: UPDATED"
} else {
    Write-Host "parse_if start: NOT FOUND"
}

if ($content.Contains($old_if_end)) {
    $content = $content.Replace($old_if_end, $new_if_end)
    Write-Host "parse_if end: UPDATED"
} else {
    Write-Host "parse_if end: NOT FOUND"
}

# 2. Update parse_method_body signature — add default_: bool, async_: bool
$old_method_body_sig = "    fn parse_method_body(`r`n        &mut self,`r`n        visibility   : Visibility,`r`n        static_      : bool,`r`n        abstract_    : bool,`r`n        override_    : bool,`r`n        inline_      : bool,`r`n        calling_conv : Option<CallingConv>,`r`n        section      : Option<String>,`r`n        name         : String,`r`n        return_ty    : Option<Type>,`r`n    ) -> ParseResult<Method> {"
$new_method_body_sig = "    fn parse_method_body(`r`n        &mut self,`r`n        visibility   : Visibility,`r`n        static_      : bool,`r`n        abstract_    : bool,`r`n        default_     : bool,`r`n        override_    : bool,`r`n        inline_      : bool,`r`n        async_       : bool,`r`n        calling_conv : Option<CallingConv>,`r`n        section      : Option<String>,`r`n        name         : String,`r`n        return_ty    : Option<Type>,`r`n    ) -> ParseResult<Method> {"

if ($content.Contains($old_method_body_sig)) {
    $content = $content.Replace($old_method_body_sig, $new_method_body_sig)
    Write-Host "parse_method_body signature: UPDATED"
} else {
    Write-Host "parse_method_body signature: NOT FOUND"
    $idx = $content.IndexOf("fn parse_method_body")
    Write-Host $content.Substring($idx, 400)
}

# 3. Update parse_method_body return — add default_ and async_
$old_return = "        Ok(Method { visibility, static_, abstract_, override_, inline_, calling_conv, section, name, params, return_ty, body })"
$new_return = "        Ok(Method { visibility, static_, abstract_, default_, override_, inline_, async_, calling_conv, section, name, params, return_ty, body })"

if ($content.Contains($old_return)) {
    $content = $content.Replace($old_return, $new_return)
    Write-Host "parse_method_body return: UPDATED"
} else {
    Write-Host "parse_method_body return: NOT FOUND"
    $idx = $content.IndexOf("Ok(Method {")
    Write-Host $content.Substring($idx, 200)
}

[System.IO.File]::WriteAllText($file, $content, [System.Text.Encoding]::UTF8)
Write-Host "File written."
