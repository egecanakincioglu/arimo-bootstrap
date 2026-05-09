// ─────────────────────────────────────────────────────────────────────────────
// Arimo Lang — CodeGen (LLVM / inkwell)
// Hedef: .arm → LLVM IR → native binary
// ─────────────────────────────────────────────────────────────────────────────

use std::collections::HashMap;
use std::path::Path;

use inkwell::context::Context;
use inkwell::builder::Builder;
use inkwell::module::Module;
use inkwell::values::{
    BasicValueEnum, FunctionValue, PointerValue, BasicValue,
};
use inkwell::types::{BasicTypeEnum, BasicType};
use inkwell::AddressSpace;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode,
    Target, TargetMachine,
};
use inkwell::OptimizationLevel;

use crate::ast::*;

// ─────────────────────────────────────────────────────────────────────────────
// Hata tipi
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct CodeGenError {
    pub message: String,
}

impl CodeGenError {
    fn new(msg: impl Into<String>) -> Self {
        CodeGenError { message: msg.into() }
    }
}

impl std::fmt::Display for CodeGenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "codegen error — {}", self.message)
    }
}

type CgResult<T> = Result<T, CodeGenError>;

// ─────────────────────────────────────────────────────────────────────────────
// Değişken bilgisi (stack slot)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct VarSlot<'ctx> {
    ptr : PointerValue<'ctx>,
    ty  : BasicTypeEnum<'ctx>,
}

// ─────────────────────────────────────────────────────────────────────────────
// CodeGen yapısı
// ─────────────────────────────────────────────────────────────────────────────

pub struct CodeGen<'ctx> {
    ctx     : &'ctx Context,
    module  : Module<'ctx>,
    builder : Builder<'ctx>,
    // Yerel değişkenler: kapsam yığını
    scopes  : Vec<HashMap<String, VarSlot<'ctx>>>,
    // Tanımlı fonksiyonlar
    fns     : HashMap<String, FunctionValue<'ctx>>,
    // Geçerli fonksiyon
    cur_fn  : Option<FunctionValue<'ctx>>,
}

impl<'ctx> CodeGen<'ctx> {
    pub fn new(ctx: &'ctx Context, module_name: &str) -> Self {
        let module  = ctx.create_module(module_name);
        let builder = ctx.create_builder();
        CodeGen {
            ctx,
            module,
            builder,
            scopes : Vec::new(),
            fns    : HashMap::new(),
            cur_fn : None,
        }
    }

    // ── Kapsam yönetimi ──────────────────────────────────────────────────────

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn define_var(&mut self, name: &str, ptr: PointerValue<'ctx>, ty: BasicTypeEnum<'ctx>) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), VarSlot { ptr, ty });
        }
    }

    fn lookup_var(&self, name: &str) -> Option<&VarSlot<'ctx>> {
        for scope in self.scopes.iter().rev() {
            if let Some(slot) = scope.get(name) {
                return Some(slot);
            }
        }
        None
    }

    // ── Tip çevirisi: Arimo → LLVM ───────────────────────────────────────────

    fn llvm_type(&self, ty: &Type) -> Option<BasicTypeEnum<'ctx>> {
        match ty {
            Type::Integer         => Some(self.ctx.i64_type().into()),
            Type::Float           => Some(self.ctx.f64_type().into()),
            Type::Boolean         => Some(self.ctx.bool_type().into()),
            Type::Str             => Some(self.ctx.ptr_type(AddressSpace::default()).into()),
            Type::Void            => None,
            Type::NoReturn        => None,
            Type::U8  | Type::I8  => Some(self.ctx.i8_type().into()),
            Type::U16 | Type::I16 => Some(self.ctx.i16_type().into()),
            Type::U32 | Type::I32 => Some(self.ctx.i32_type().into()),
            Type::U64 | Type::I64 => Some(self.ctx.i64_type().into()),
            Type::RawPtr(_)       => Some(self.ctx.ptr_type(AddressSpace::default()).into()),
            Type::Nullable(inner) => self.llvm_type(inner),
            Type::Named(_)        => Some(self.ctx.ptr_type(AddressSpace::default()).into()),
            Type::FnPtr(params, ret) => {
                let ret_llvm = self.llvm_type(ret.as_ref());
                let params_llvm: Vec<inkwell::types::BasicMetadataTypeEnum> = params.iter()
                    .filter_map(|p| self.llvm_type(p))
                    .map(|t| t.into())
                    .collect();
                let fn_ty = match ret_llvm {
                    Some(r) => r.fn_type(&params_llvm, false),
                    None    => self.ctx.void_type().fn_type(&params_llvm, false),
                };
                Some(fn_ty.ptr_type(AddressSpace::default()).into())
            }
            _ => Some(self.ctx.ptr_type(AddressSpace::default()).into()),
        }
    }

    // ── Modülü işle ──────────────────────────────────────────────────────────

    pub fn compile_module(&mut self, module: &crate::ast::Module) -> CgResult<()> {
        // Önce extern fonksiyonları kayıt et
        for item in &module.items {
            if let Item::Extern(ext) = item {
                self.declare_extern_block(ext)?;
            }
        }

        // printf / IO altyapısı — her zaman lazım
        self.declare_printf();

        // İki geçiş: önce fonksiyon imzaları, sonra gövdeler
        // Geçiş 1: class'lardaki static metodları kayıt et
        for item in &module.items {
            if let Item::Class(c) = item {
                self.register_class_methods(c)?;
            }
        }
        // Geçiş 2: gövdeleri üret
        for item in &module.items {
            if let Item::Class(c) = item {
                self.compile_class(c)?;
            }
        }
        Ok(())
    }

    // ── printf extern tanımı ─────────────────────────────────────────────────

    fn declare_printf(&mut self) {
        if self.module.get_function("printf").is_some() { return; }
        let i8_ptr = self.ctx.ptr_type(AddressSpace::default());
        let i32_ty = self.ctx.i32_type();
        let printf_ty = i32_ty.fn_type(&[i8_ptr.into()], true);
        self.module.add_function("printf", printf_ty, None);
    }

    // ── Extern "C" blokları ──────────────────────────────────────────────────

    fn declare_extern_block(&mut self, block: &ExternBlock) -> CgResult<()> {
        for decl in &block.decls {
            if self.module.get_function(&decl.name).is_some() { continue; }
            let params: Vec<inkwell::types::BasicMetadataTypeEnum> = decl.params.iter()
                .filter_map(|p| self.llvm_type(&p.ty).map(|t| t.into()))
                .collect();
            let fn_ty = match &decl.return_ty {
                Some(ret) => match self.llvm_type(ret) {
                    Some(rt) => rt.fn_type(&params, decl.variadic),
                    None     => self.ctx.void_type().fn_type(&params, decl.variadic),
                },
                None => self.ctx.void_type().fn_type(&params, decl.variadic),
            };
            self.module.add_function(&decl.name, fn_ty, None);
        }
        Ok(())
    }

    // ── Class metodlarını kayıt et (imza) ────────────────────────────────────

    fn register_class_methods(&mut self, c: &ClassDecl) -> CgResult<()> {
        for m in &c.methods {
            let fn_name = format!("{}_{}", c.name, m.name);
            if self.module.get_function(&fn_name).is_some() { continue; }

            let mut param_types: Vec<inkwell::types::BasicMetadataTypeEnum> = Vec::new();
            // Instance metodlar için ilk parametre 'this' pointer
            if !m.static_ {
                param_types.push(self.ctx.ptr_type(AddressSpace::default()).into());
            }
            for p in &m.params {
                if let Some(t) = self.llvm_type(&p.ty) {
                    param_types.push(t.into());
                }
            }

            let fn_val = match &m.return_ty {
                Some(ret_ty) => match self.llvm_type(ret_ty) {
                    Some(rt) => {
                        let fn_ty = rt.fn_type(&param_types, false);
                        self.module.add_function(&fn_name, fn_ty, None)
                    }
                    None => {
                        let fn_ty = self.ctx.void_type().fn_type(&param_types, false);
                        self.module.add_function(&fn_name, fn_ty, None)
                    }
                },
                None => {
                    let fn_ty = self.ctx.void_type().fn_type(&param_types, false);
                    self.module.add_function(&fn_name, fn_ty, None)
                }
            };
            self.fns.insert(fn_name, fn_val);
        }
        Ok(())
    }

    // ── Class gövdelerini derle ───────────────────────────────────────────────

    fn compile_class(&mut self, c: &ClassDecl) -> CgResult<()> {
        for m in &c.methods {
            if m.body.is_none() { continue; }
            self.compile_method(c, m)?;
        }
        Ok(())
    }

    // ── Metod gövdesi ─────────────────────────────────────────────────────────

    fn compile_method(&mut self, c: &ClassDecl, m: &Method) -> CgResult<()> {
        let fn_name = format!("{}_{}", c.name, m.name);

        // main() → LLVM main fonksiyonu
        let is_entry = m.name == "main" && m.static_;

        let fn_val = if is_entry {
            // main fonksiyonunu özel olarak tanımla: i32 main()
            let main_ty = self.ctx.i32_type().fn_type(&[], false);
            if let Some(existing) = self.module.get_function("main") {
                existing
            } else {
                self.module.add_function("main", main_ty, None)
            }
        } else {
            match self.fns.get(&fn_name).copied() {
                Some(f) => f,
                None => return Err(CodeGenError::new(format!("function not registered: {}", fn_name))),
            }
        };

        let body = m.body.as_ref().unwrap();

        let entry_block = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry_block);

        self.cur_fn = Some(fn_val);
        self.push_scope();

        // Parametreleri kapsama ekle
        let param_offset = if m.static_ { 0 } else { 1 };
        for (i, p) in m.params.iter().enumerate() {
            if let Some(llvm_ty) = self.llvm_type(&p.ty) {
                let param_val = fn_val.get_nth_param((i + param_offset) as u32);
                if let Some(pv) = param_val {
                    let alloca = self.builder.build_alloca(llvm_ty, &p.name)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_store(alloca, pv)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.define_var(&p.name, alloca, llvm_ty);
                }
            }
        }

        // Statement'ları derle
        let mut returned = false;
        for stmt in body {
            if self.compile_stmt(stmt)? {
                returned = true;
                break;
            }
        }

        // Return yoksa otomatik ekle
        if !returned {
            if is_entry {
                let zero = self.ctx.i32_type().const_int(0, false);
                self.builder.build_return(Some(&zero))
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
            } else {
                self.builder.build_return(None)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
            }
        }

        self.pop_scope();
        self.cur_fn = None;

        // LLVM doğrulaması
        if fn_val.verify(true) {
            Ok(())
        } else {
            Err(CodeGenError::new(format!("LLVM function verification failed: {}", fn_name)))
        }
    }

    // ── Statement'lar → LLVM IR ───────────────────────────────────────────────
    // Dönüş değeri: true = bu statement bir return içeriyor

    fn compile_stmt(&mut self, stmt: &Stmt) -> CgResult<bool> {
        match stmt {
            Stmt::Return(expr) => {
                match expr {
                    None => {
                        // main() için i32 0 döndür
                        let cur = self.cur_fn.unwrap();
                        if cur.get_type().get_return_type().is_some() {
                            let zero = self.ctx.i32_type().const_int(0, false);
                            self.builder.build_return(Some(&zero))
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                        } else {
                            self.builder.build_return(None)
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                        }
                    }
                    Some(e) => {
                        let val = self.compile_expr(e)?;
                        match val {
                            Some(v) => {
                                self.builder.build_return(Some(&v))
                                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                            }
                            None => {
                                self.builder.build_return(None)
                                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                            }
                        }
                    }
                }
                Ok(true)
            }

            Stmt::VarDecl { ty, name, value, .. } => {
                if let Some(llvm_ty) = self.llvm_type(ty) {
                    let alloca = self.builder.build_alloca(llvm_ty, name)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    if let Some(init_expr) = value {
                        if let Some(val) = self.compile_expr(init_expr)? {
                            let coerced = self.coerce_value(val, llvm_ty)?;
                            self.builder.build_store(alloca, coerced)
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                        }
                    }
                    self.define_var(name, alloca, llvm_ty);
                }
                Ok(false)
            }

            Stmt::ExprStmt(e) => {
                self.compile_expr(e)?;
                Ok(false)
            }

            Stmt::If { hint: _, cond, then, else_if, else_ } => {
                self.compile_if(cond, then, else_if, else_)
            }

            Stmt::While { cond, body } => {
                self.compile_while(cond, body)
            }

            Stmt::For { init, cond, step, body } => {
                self.compile_for(init, cond, step, body)
            }

            Stmt::ForEach { ty, name, iter, body } => {
                // ForEach: koleksiyonlar üzerinde — şimdilik pass
                let _ = (ty, name, iter, body);
                Ok(false)
            }

            Stmt::Block(stmts) => {
                self.push_scope();
                let mut returned = false;
                for s in stmts {
                    if self.compile_stmt(s)? {
                        returned = true;
                        break;
                    }
                }
                self.pop_scope();
                Ok(returned)
            }

            Stmt::TryCatch { try_body, catches: _, finally_body } => {
                // Basit implementasyon: try bloğunu doğrudan çalıştır
                let mut ret = false;
                self.push_scope();
                for s in try_body {
                    if self.compile_stmt(s)? { ret = true; break; }
                }
                self.pop_scope();
                if let Some(fin) = finally_body {
                    self.push_scope();
                    for s in fin {
                        if self.compile_stmt(s)? { break; }
                    }
                    self.pop_scope();
                }
                Ok(ret)
            }

            Stmt::Throw(_) => {
                // Şimdilik: abort() çağır
                self.builder.build_unreachable()
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(true)
            }

            Stmt::Switch { expr, cases } => {
                self.compile_switch(expr, cases)
            }

            Stmt::Asm(_) | Stmt::Defer(_) => Ok(false),
            Stmt::Break | Stmt::Continue   => Ok(false),
        }
    }

    // ── if / else ────────────────────────────────────────────────────────────

    fn compile_if(
        &mut self,
        cond    : &Expr,
        then    : &[Stmt],
        else_if : &[(Expr, Vec<Stmt>)],
        else_   : &Option<Vec<Stmt>>,
    ) -> CgResult<bool> {
        let cur_fn = self.cur_fn.unwrap();
        let cond_val = self.compile_expr(cond)?
            .ok_or_else(|| CodeGenError::new("if condition has no value"))?;
        let cond_bool = self.to_bool(cond_val)?;

        let then_bb  = self.ctx.append_basic_block(cur_fn, "if.then");
        let merge_bb = self.ctx.append_basic_block(cur_fn, "if.merge");
        let else_bb  = if else_.is_some() || !else_if.is_empty() {
            self.ctx.append_basic_block(cur_fn, "if.else")
        } else {
            merge_bb
        };

        self.builder.build_conditional_branch(cond_bool, then_bb, else_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // then bloğu
        self.builder.position_at_end(then_bb);
        self.push_scope();
        let mut then_returned = false;
        for s in then {
            if self.compile_stmt(s)? { then_returned = true; break; }
        }
        self.pop_scope();
        if !then_returned {
            self.builder.build_unconditional_branch(merge_bb)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }

        // else / else-if bloğu
        let mut else_returned = false;
        if else_bb != merge_bb {
            self.builder.position_at_end(else_bb);
            self.push_scope();
            if let Some(eb) = else_ {
                for s in eb {
                    if self.compile_stmt(s)? { else_returned = true; break; }
                }
            }
            self.pop_scope();
            if !else_returned {
                self.builder.build_unconditional_branch(merge_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
            }
        } else {
            // else bloğu yok → else_returned = false (merge'e düşüyor)
        }

        self.builder.position_at_end(merge_bb);

        // Her iki dal da return yaptıysa merge block ulaşılamaz
        if then_returned && else_returned {
            self.builder.build_unreachable()
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            return Ok(true);
        }

        Ok(false)
    }

    // ── while döngüsü ────────────────────────────────────────────────────────

    fn compile_while(&mut self, cond: &Expr, body: &[Stmt]) -> CgResult<bool> {
        let cur_fn  = self.cur_fn.unwrap();
        let cond_bb = self.ctx.append_basic_block(cur_fn, "while.cond");
        let body_bb = self.ctx.append_basic_block(cur_fn, "while.body");
        let exit_bb = self.ctx.append_basic_block(cur_fn, "while.exit");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(cond_bb);
        let cond_val = self.compile_expr(cond)?
            .ok_or_else(|| CodeGenError::new("while condition has no value"))?;
        let cond_bool = self.to_bool(cond_val)?;
        self.builder.build_conditional_branch(cond_bool, body_bb, exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(body_bb);
        self.push_scope();
        for s in body {
            if self.compile_stmt(s)? { break; }
        }
        self.pop_scope();
        if !self.current_block_terminated() {
            self.builder.build_unconditional_branch(cond_bb)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }

        self.builder.position_at_end(exit_bb);
        Ok(false)
    }

    // ── for döngüsü ──────────────────────────────────────────────────────────

    fn compile_for(
        &mut self,
        init : &Stmt,
        cond : &Expr,
        step : &Expr,
        body : &[Stmt],
    ) -> CgResult<bool> {
        let cur_fn  = self.cur_fn.unwrap();
        self.push_scope();
        self.compile_stmt(init)?;

        let cond_bb = self.ctx.append_basic_block(cur_fn, "for.cond");
        let body_bb = self.ctx.append_basic_block(cur_fn, "for.body");
        let step_bb = self.ctx.append_basic_block(cur_fn, "for.step");
        let exit_bb = self.ctx.append_basic_block(cur_fn, "for.exit");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(cond_bb);
        let cv = self.compile_expr(cond)?
            .ok_or_else(|| CodeGenError::new("for condition has no value"))?;
        let cb = self.to_bool(cv)?;
        self.builder.build_conditional_branch(cb, body_bb, exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(body_bb);
        for s in body {
            if self.compile_stmt(s)? { break; }
        }
        if !self.current_block_terminated() {
            self.builder.build_unconditional_branch(step_bb)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }

        self.builder.position_at_end(step_bb);
        self.compile_expr(step)?;
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(exit_bb);
        self.pop_scope();
        Ok(false)
    }

    // ── switch ───────────────────────────────────────────────────────────────

    fn compile_switch(&mut self, expr: &Expr, cases: &[SwitchCase]) -> CgResult<bool> {
        // switch → if/else zinciri olarak compile et
        let val = self.compile_expr(expr)?;
        let cur_fn = self.cur_fn.unwrap();
        let exit_bb = self.ctx.append_basic_block(cur_fn, "switch.exit");

        for case in cases {
            let case_val = self.compile_expr(&case.pattern)?;
            if let (Some(v), Some(cv)) = (val.as_ref(), case_val) {
                let eq = self.build_eq(*v, cv)?;
                let then_bb = self.ctx.append_basic_block(cur_fn, "case.body");
                let next_bb = self.ctx.append_basic_block(cur_fn, "case.next");
                self.builder.build_conditional_branch(eq, then_bb, next_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.position_at_end(then_bb);
                self.push_scope();
                let mut ret = false;
                for s in &case.body {
                    if self.compile_stmt(s)? { ret = true; break; }
                }
                self.pop_scope();
                if !ret {
                    self.builder.build_unconditional_branch(exit_bb)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                }
                self.builder.position_at_end(next_bb);
            }
        }
        self.builder.build_unconditional_branch(exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(exit_bb);
        Ok(false)
    }

    // ── Expression → LLVM IR ─────────────────────────────────────────────────

    fn compile_expr(&mut self, expr: &Expr) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        match expr {
            // ── Literaller ────────────────────────────────────────────────────
            Expr::IntLit(n) => {
                let v = self.ctx.i64_type().const_int(*n as u64, *n < 0);
                Ok(Some(v.into()))
            }
            Expr::FloatLit(f) => {
                let v = self.ctx.f64_type().const_float(*f);
                Ok(Some(v.into()))
            }
            Expr::BoolLit(b) => {
                let v = self.ctx.bool_type().const_int(*b as u64, false);
                Ok(Some(v.into()))
            }
            Expr::StrLit(s) => {
                let g = self.build_global_string(s)?;
                Ok(Some(g.into()))
            }
            Expr::StrInterp(parts) => {
                // String interpolation: şimdilik sadece ilk metin kısmını al
                let mut result = String::new();
                for part in parts {
                    match part {
                        StringPart::Text(t) => result.push_str(t),
                        StringPart::Interp(_) => result.push_str("%s"),
                    }
                }
                let g = self.build_global_string(&result)?;
                Ok(Some(g.into()))
            }
            Expr::NullLit => {
                let v = self.ctx.ptr_type(AddressSpace::default()).const_null();
                Ok(Some(v.into()))
            }

            // ── Değişken okuma ────────────────────────────────────────────────
            Expr::Ident(name) => {
                if let Some(slot) = self.lookup_var(name).cloned() {
                    let v = self.builder.build_load(slot.ty, slot.ptr, name)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    Ok(Some(v))
                } else {
                    Ok(None)
                }
            }

            // ── IO.print() ────────────────────────────────────────────────────
            Expr::StaticCall { class, method, args }
                if class == "IO" && method == "print" =>
            {
                self.compile_io_print(args)?;
                Ok(None)
            }

            // ── Diğer static çağrılar ─────────────────────────────────────────
            Expr::StaticCall { class, method, args } => {
                self.compile_static_call(class, method, args)
            }

            // ── Instance metod çağrısı ─────────────────────────────────────────
            Expr::MethodCall { object, method, args } => {
                self.compile_method_call(object, method, args)
            }

            // ── Constructor ───────────────────────────────────────────────────
            Expr::ConstructorCall { class: _, args } => {
                // Şimdilik argümanları derle ve None döndür
                for a in args { self.compile_expr(a)?; }
                Ok(None)
            }

            // ── Binary operatörler ────────────────────────────────────────────
            Expr::BinOp { op, left, right } => {
                self.compile_binop(op, left, right)
            }

            // ── Unary operatörler ─────────────────────────────────────────────
            Expr::UnaryOp { op, expr } => {
                self.compile_unary(op, expr)
            }

            // ── Type cast ─────────────────────────────────────────────────────
            Expr::Cast { expr, ty } => {
                let val = self.compile_expr(expr)?;
                if let (Some(v), Some(target_ty)) = (val, self.llvm_type(ty)) {
                    let casted = self.build_cast(v, target_ty)?;
                    Ok(Some(casted))
                } else {
                    Ok(None)
                }
            }

            // ── Ternary ───────────────────────────────────────────────────────
            Expr::Ternary { cond, then, else_ } => {
                let cur_fn   = self.cur_fn.unwrap();
                let cond_val = self.compile_expr(cond)?
                    .ok_or_else(|| CodeGenError::new("ternary condition empty"))?;
                let cond_b   = self.to_bool(cond_val)?;
                let then_bb  = self.ctx.append_basic_block(cur_fn, "tern.then");
                let else_bb  = self.ctx.append_basic_block(cur_fn, "tern.else");
                let merge_bb = self.ctx.append_basic_block(cur_fn, "tern.merge");
                self.builder.build_conditional_branch(cond_b, then_bb, else_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.position_at_end(then_bb);
                let then_val = self.compile_expr(then)?;
                self.builder.build_unconditional_branch(merge_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let then_end = self.builder.get_insert_block().unwrap();
                self.builder.position_at_end(else_bb);
                let else_val = self.compile_expr(else_)?;
                self.builder.build_unconditional_branch(merge_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let else_end = self.builder.get_insert_block().unwrap();
                self.builder.position_at_end(merge_bb);
                if let (Some(tv), Some(ev)) = (then_val, else_val) {
                    let phi = self.builder.build_phi(tv.get_type(), "tern.phi")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    phi.add_incoming(&[(&tv, then_end), (&ev, else_end)]);
                    Ok(Some(phi.as_basic_value()))
                } else {
                    Ok(None)
                }
            }

            // ── Index ─────────────────────────────────────────────────────────
            Expr::Index { object, index } => {
                let obj = self.compile_expr(object)?;
                let idx = self.compile_expr(index)?;
                if let (Some(o), Some(i)) = (obj, idx) {
                    // Array element: unsafe GEP
                    let i64_idx = self.to_i64(i)?;
                    if let BasicValueEnum::PointerValue(ptr) = o {
                        let elem_ty = self.ctx.i64_type();
                        let gep = unsafe {
                            self.builder.build_gep(elem_ty, ptr, &[i64_idx], "arr.elem")
                                .map_err(|e| CodeGenError::new(e.to_string()))?
                        };
                        let v = self.builder.build_load(BasicTypeEnum::IntType(elem_ty), gep, "arr.load")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        return Ok(Some(v));
                    }
                }
                Ok(None)
            }

            // ── this / super ─────────────────────────────────────────────────
            Expr::This | Expr::Super => Ok(None),

            // ── await ─────────────────────────────────────────────────────────
            Expr::Await(inner) => self.compile_expr(inner),

            // ── Match ─────────────────────────────────────────────────────────
            Expr::Match { expr, arms } => {
                // Şimdilik basit: sadece ilk arm
                let _ = (expr, arms);
                Ok(None)
            }

            // ── Lambda ───────────────────────────────────────────────────────
            Expr::Lambda { .. } => Ok(None),

            // ── FieldAccess ───────────────────────────────────────────────────
            Expr::FieldAccess { .. } | Expr::NullSafeAccess { .. } => Ok(None),
        }
    }

    // ── IO.print() ───────────────────────────────────────────────────────────

    fn compile_io_print(&mut self, args: &[Expr]) -> CgResult<()> {
        let printf = self.module.get_function("printf")
            .ok_or_else(|| CodeGenError::new("printf not declared"))?;

        if args.is_empty() {
            let fmt = self.build_global_string("\n")?;
            self.builder.build_call(printf, &[fmt.into()], "print")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            return Ok(());
        }

        // İlk argümanı format string olarak kullan
        let fmt_val = self.compile_expr(&args[0])?
            .ok_or_else(|| CodeGenError::new("IO.print: no format string"))?;

        // Newline eklenmiş format string
        let fmt_with_nl = self.build_global_string_with_nl(&args[0])?;

        let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum> = vec![fmt_with_nl.into()];
        for a in &args[1..] {
            if let Some(v) = self.compile_expr(a)? {
                call_args.push(v.into());
            }
        }
        let _ = fmt_val; // suppress warning
        self.builder.build_call(printf, &call_args, "print")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        Ok(())
    }

    fn build_global_string_with_nl(&mut self, expr: &Expr) -> CgResult<PointerValue<'ctx>> {
        let s = match expr {
            Expr::StrLit(s) => format!("{}\n", s),
            Expr::StrInterp(parts) => {
                let mut r = String::new();
                for p in parts {
                    match p {
                        StringPart::Text(t) => r.push_str(t),
                        StringPart::Interp(_) => r.push_str("%s"),
                    }
                }
                format!("{}\n", r)
            }
            _ => "\n".to_string(),
        };
        self.build_global_string(&s)
    }

    // ── Static çağrı ─────────────────────────────────────────────────────────

    fn compile_static_call(
        &mut self,
        class  : &str,
        method : &str,
        args   : &[Expr],
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        let fn_name = format!("{}_{}", class, method);
        let compiled_args: Vec<BasicValueEnum<'ctx>> = args.iter()
            .filter_map(|a| self.compile_expr(a).ok().flatten())
            .collect();

        if let Some(fn_val) = self.fns.get(&fn_name).copied()
            .or_else(|| self.module.get_function(&fn_name))
        {
            let meta_args: Vec<inkwell::values::BasicMetadataValueEnum> =
                compiled_args.iter().map(|v| (*v).into()).collect();
            let call = self.builder.build_call(fn_val, &meta_args, "call")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            Ok(call.try_as_basic_value().basic())
        } else {
            Ok(None)
        }
    }

    // ── Instance metod çağrısı ────────────────────────────────────────────────

    fn compile_method_call(
        &mut self,
        _object : &Expr,
        _method : &str,
        args    : &[Expr],
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        for a in args { self.compile_expr(a)?; }
        Ok(None)
    }

    // ── Binary operatörler ────────────────────────────────────────────────────

    fn compile_binop(
        &mut self,
        op    : &BinOp,
        left  : &Expr,
        right : &Expr,
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        // Atama özel işlenir
        if matches!(op, BinOp::Assign) {
            return self.compile_assign(left, right);
        }
        // Compound assignment
        if matches!(op, BinOp::AddAssign | BinOp::SubAssign | BinOp::MulAssign | BinOp::DivAssign) {
            return self.compile_compound_assign(op, left, right);
        }

        let lv = self.compile_expr(left)?;
        let rv = self.compile_expr(right)?;

        let (lv, rv) = match (lv, rv) {
            (Some(l), Some(r)) => (l, r),
            _ => return Ok(None),
        };

        let result: BasicValueEnum<'ctx> = match op {
            BinOp::Add => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    self.builder.build_int_add(l, r, "add")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                (BasicValueEnum::FloatValue(l), BasicValueEnum::FloatValue(r)) => {
                    self.builder.build_float_add(l, r, "fadd")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            BinOp::Sub => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    self.builder.build_int_sub(l, r, "sub")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                (BasicValueEnum::FloatValue(l), BasicValueEnum::FloatValue(r)) => {
                    self.builder.build_float_sub(l, r, "fsub")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            BinOp::Mul => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    self.builder.build_int_mul(l, r, "mul")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                (BasicValueEnum::FloatValue(l), BasicValueEnum::FloatValue(r)) => {
                    self.builder.build_float_mul(l, r, "fmul")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            BinOp::Div => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    self.builder.build_int_signed_div(l, r, "div")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                (BasicValueEnum::FloatValue(l), BasicValueEnum::FloatValue(r)) => {
                    self.builder.build_float_div(l, r, "fdiv")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            BinOp::Mod => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    self.builder.build_int_signed_rem(l, r, "rem")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            BinOp::Eq  => { let r = self.build_eq(lv, rv)?;  return Ok(Some(r.into())); }
            BinOp::Ne  => { let r = self.build_ne(lv, rv)?;  return Ok(Some(r.into())); }
            BinOp::Lt  => { let r = self.build_lt(lv, rv)?;  return Ok(Some(r.into())); }
            BinOp::Le  => { let r = self.build_le(lv, rv)?;  return Ok(Some(r.into())); }
            BinOp::Gt  => { let r = self.build_gt(lv, rv)?;  return Ok(Some(r.into())); }
            BinOp::Ge  => { let r = self.build_ge(lv, rv)?;  return Ok(Some(r.into())); }
            BinOp::And => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    self.builder.build_and(l, r, "and")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            BinOp::Or => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    self.builder.build_or(l, r, "or")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            BinOp::BitAnd => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    self.builder.build_and(l, r, "band")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            BinOp::BitOr => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    self.builder.build_or(l, r, "bor")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            BinOp::BitXor => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    self.builder.build_xor(l, r, "xor")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            BinOp::Shl => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    self.builder.build_left_shift(l, r, "shl")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            BinOp::Shr => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    self.builder.build_right_shift(l, r, false, "shr")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            _ => return Ok(None),
        };
        Ok(Some(result))
    }

    fn compile_assign(&mut self, target: &Expr, value: &Expr) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        let val = self.compile_expr(value)?;
        if let Some(v) = val {
            match target {
                Expr::Ident(name) => {
                    if let Some(slot) = self.lookup_var(name).cloned() {
                        let coerced = self.coerce_value(v, slot.ty)?;
                        self.builder.build_store(slot.ptr, coerced)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                    }
                }
                _ => {}
            }
            Ok(Some(v))
        } else {
            Ok(None)
        }
    }

    fn compile_compound_assign(
        &mut self,
        op     : &BinOp,
        target : &Expr,
        value  : &Expr,
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        let bin_op = match op {
            BinOp::AddAssign => BinOp::Add,
            BinOp::SubAssign => BinOp::Sub,
            BinOp::MulAssign => BinOp::Mul,
            BinOp::DivAssign => BinOp::Div,
            _                => return Ok(None),
        };
        let computed = Expr::BinOp {
            op    : bin_op,
            left  : Box::new(target.clone()),
            right : Box::new(value.clone()),
        };
        self.compile_assign(target, &computed)
    }

    // ── Unary operatörler ─────────────────────────────────────────────────────

    fn compile_unary(&mut self, op: &UnaryOp, expr: &Expr) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        let val = self.compile_expr(expr)?;
        let val = match val { Some(v) => v, None => return Ok(None) };
        match op {
            UnaryOp::Neg => match val {
                BasicValueEnum::IntValue(v) => {
                    let r = self.builder.build_int_neg(v, "neg")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    Ok(Some(r.into()))
                }
                BasicValueEnum::FloatValue(v) => {
                    let r = self.builder.build_float_neg(v, "fneg")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    Ok(Some(r.into()))
                }
                _ => Ok(None),
            },
            UnaryOp::Not => match val {
                BasicValueEnum::IntValue(v) => {
                    let r = self.builder.build_not(v, "not")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    Ok(Some(r.into()))
                }
                _ => Ok(None),
            },
            UnaryOp::BitNot => match val {
                BasicValueEnum::IntValue(v) => {
                    let r = self.builder.build_not(v, "bitnot")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    Ok(Some(r.into()))
                }
                _ => Ok(None),
            },
            UnaryOp::PreInc | UnaryOp::PostInc => {
                let one = self.ctx.i64_type().const_int(1, false);
                self.compile_assign(expr, &Expr::BinOp {
                    op    : BinOp::Add,
                    left  : Box::new(expr.clone()),
                    right : Box::new(Expr::IntLit(1)),
                })?;
                if matches!(op, UnaryOp::PostInc) { Ok(Some(val)) }
                else {
                    let _ = one;
                    self.compile_expr(expr)
                }
            }
            UnaryOp::PreDec | UnaryOp::PostDec => {
                self.compile_assign(expr, &Expr::BinOp {
                    op    : BinOp::Sub,
                    left  : Box::new(expr.clone()),
                    right : Box::new(Expr::IntLit(1)),
                })?;
                if matches!(op, UnaryOp::PostDec) { Ok(Some(val)) }
                else { self.compile_expr(expr) }
            }
        }
    }

    // ── Yardımcılar ──────────────────────────────────────────────────────────

    fn build_global_string(&mut self, s: &str) -> CgResult<PointerValue<'ctx>> {
        let gs = self.builder.build_global_string_ptr(s, "str")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        Ok(gs.as_pointer_value())
    }

    fn to_bool(&mut self, val: BasicValueEnum<'ctx>)
        -> CgResult<inkwell::values::IntValue<'ctx>>
    {
        match val {
            BasicValueEnum::IntValue(v) => {
                let bty = self.ctx.bool_type();
                if v.get_type().get_bit_width() == 1 {
                    Ok(v)
                } else {
                    let zero = v.get_type().const_int(0, false);
                    self.builder.build_int_compare(
                        inkwell::IntPredicate::NE, v, zero, "tobool"
                    ).map_err(|e| CodeGenError::new(e.to_string()))
                }
            }
            BasicValueEnum::PointerValue(p) => {
                let null = p.get_type().const_null();
                self.builder.build_int_compare(
                    inkwell::IntPredicate::NE, p, null, "ptrbool"
                ).map_err(|e| CodeGenError::new(e.to_string()))
            }
            _ => Err(CodeGenError::new("cannot convert to bool")),
        }
    }

    fn to_i64(&mut self, val: BasicValueEnum<'ctx>)
        -> CgResult<inkwell::values::IntValue<'ctx>>
    {
        match val {
            BasicValueEnum::IntValue(v) => {
                let i64ty = self.ctx.i64_type();
                if v.get_type().get_bit_width() == 64 {
                    Ok(v)
                } else {
                    self.builder.build_int_z_extend(v, i64ty, "zext")
                        .map_err(|e| CodeGenError::new(e.to_string()))
                }
            }
            _ => Err(CodeGenError::new("cannot convert to i64")),
        }
    }

    fn build_eq(&mut self, l: BasicValueEnum<'ctx>, r: BasicValueEnum<'ctx>)
        -> CgResult<inkwell::values::IntValue<'ctx>>
    {
        match (l, r) {
            (BasicValueEnum::IntValue(a), BasicValueEnum::IntValue(b)) =>
                self.builder.build_int_compare(inkwell::IntPredicate::EQ, a, b, "eq")
                    .map_err(|e| CodeGenError::new(e.to_string())),
            (BasicValueEnum::FloatValue(a), BasicValueEnum::FloatValue(b)) =>
                self.builder.build_float_compare(inkwell::FloatPredicate::OEQ, a, b, "feq")
                    .map_err(|e| CodeGenError::new(e.to_string())),
            _ => Ok(self.ctx.bool_type().const_int(0, false)),
        }
    }

    fn build_ne(&mut self, l: BasicValueEnum<'ctx>, r: BasicValueEnum<'ctx>)
        -> CgResult<inkwell::values::IntValue<'ctx>>
    {
        match (l, r) {
            (BasicValueEnum::IntValue(a), BasicValueEnum::IntValue(b)) =>
                self.builder.build_int_compare(inkwell::IntPredicate::NE, a, b, "ne")
                    .map_err(|e| CodeGenError::new(e.to_string())),
            _ => Ok(self.ctx.bool_type().const_int(1, false)),
        }
    }

    fn build_lt(&mut self, l: BasicValueEnum<'ctx>, r: BasicValueEnum<'ctx>)
        -> CgResult<inkwell::values::IntValue<'ctx>>
    {
        match (l, r) {
            (BasicValueEnum::IntValue(a), BasicValueEnum::IntValue(b)) =>
                self.builder.build_int_compare(inkwell::IntPredicate::SLT, a, b, "lt")
                    .map_err(|e| CodeGenError::new(e.to_string())),
            (BasicValueEnum::FloatValue(a), BasicValueEnum::FloatValue(b)) =>
                self.builder.build_float_compare(inkwell::FloatPredicate::OLT, a, b, "flt")
                    .map_err(|e| CodeGenError::new(e.to_string())),
            _ => Ok(self.ctx.bool_type().const_int(0, false)),
        }
    }

    fn build_le(&mut self, l: BasicValueEnum<'ctx>, r: BasicValueEnum<'ctx>)
        -> CgResult<inkwell::values::IntValue<'ctx>>
    {
        match (l, r) {
            (BasicValueEnum::IntValue(a), BasicValueEnum::IntValue(b)) =>
                self.builder.build_int_compare(inkwell::IntPredicate::SLE, a, b, "le")
                    .map_err(|e| CodeGenError::new(e.to_string())),
            _ => Ok(self.ctx.bool_type().const_int(0, false)),
        }
    }

    fn build_gt(&mut self, l: BasicValueEnum<'ctx>, r: BasicValueEnum<'ctx>)
        -> CgResult<inkwell::values::IntValue<'ctx>>
    {
        match (l, r) {
            (BasicValueEnum::IntValue(a), BasicValueEnum::IntValue(b)) =>
                self.builder.build_int_compare(inkwell::IntPredicate::SGT, a, b, "gt")
                    .map_err(|e| CodeGenError::new(e.to_string())),
            _ => Ok(self.ctx.bool_type().const_int(0, false)),
        }
    }

    fn build_ge(&mut self, l: BasicValueEnum<'ctx>, r: BasicValueEnum<'ctx>)
        -> CgResult<inkwell::values::IntValue<'ctx>>
    {
        match (l, r) {
            (BasicValueEnum::IntValue(a), BasicValueEnum::IntValue(b)) =>
                self.builder.build_int_compare(inkwell::IntPredicate::SGE, a, b, "ge")
                    .map_err(|e| CodeGenError::new(e.to_string())),
            _ => Ok(self.ctx.bool_type().const_int(0, false)),
        }
    }

    fn coerce_value(
        &mut self,
        val     : BasicValueEnum<'ctx>,
        target  : BasicTypeEnum<'ctx>,
    ) -> CgResult<BasicValueEnum<'ctx>> {
        if val.get_type() == target { return Ok(val); }
        match (val, target) {
            (BasicValueEnum::IntValue(v), BasicTypeEnum::IntType(t)) => {
                let bits_src = v.get_type().get_bit_width();
                let bits_dst = t.get_bit_width();
                if bits_src < bits_dst {
                    Ok(self.builder.build_int_z_extend(v, t, "zext")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into())
                } else if bits_src > bits_dst {
                    Ok(self.builder.build_int_truncate(v, t, "trunc")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into())
                } else {
                    Ok(val)
                }
            }
            (BasicValueEnum::IntValue(v), BasicTypeEnum::FloatType(t)) => {
                Ok(self.builder.build_signed_int_to_float(v, t, "itof")
                    .map_err(|e| CodeGenError::new(e.to_string()))?.into())
            }
            _ => Ok(val),
        }
    }

    fn build_cast(
        &mut self,
        val     : BasicValueEnum<'ctx>,
        target  : BasicTypeEnum<'ctx>,
    ) -> CgResult<BasicValueEnum<'ctx>> {
        self.coerce_value(val, target)
    }

    fn current_block_terminated(&self) -> bool {
        if let Some(bb) = self.builder.get_insert_block() {
            bb.get_terminator().is_some()
        } else {
            false
        }
    }

    // ── LLVM IR çıktısı ve native binary üretimi ─────────────────────────────

    pub fn emit_ir(&self) -> String {
        self.module.print_to_string().to_string()
    }

    pub fn emit_object_file(&self, path: &Path) -> CgResult<()> {
        Target::initialize_x86(&InitializationConfig::default());

        let triple   = TargetMachine::get_default_triple();
        let target   = Target::from_triple(&triple)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let cpu      = TargetMachine::get_host_cpu_name();
        let features = TargetMachine::get_host_cpu_features();

        let machine = target.create_target_machine(
            &triple,
            cpu.to_str().unwrap_or("generic"),
            features.to_str().unwrap_or(""),
            OptimizationLevel::Default,
            RelocMode::Default,
            CodeModel::Default,
        ).ok_or_else(|| CodeGenError::new("could not create target machine"))?;

        machine.write_to_file(&self.module, FileType::Object, path)
            .map_err(|e| CodeGenError::new(e.to_string()))
    }

    pub fn verify_module(&self) -> CgResult<()> {
        self.module.verify()
            .map_err(|e| CodeGenError::new(e.to_string()))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Giriş noktası: .arm → .o üretimi
// ─────────────────────────────────────────────────────────────────────────────

pub fn compile_to_object(
    module_ast  : &crate::ast::Module,
    module_name : &str,
    out_path    : &Path,
) -> Result<(), CodeGenError> {
    let ctx = Context::create();
    let mut cg = CodeGen::new(&ctx, module_name);
    cg.compile_module(module_ast)?;
    cg.verify_module()?;
    cg.emit_object_file(out_path)
}
