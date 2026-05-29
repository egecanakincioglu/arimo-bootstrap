/*
Arimo Lang - A modern programming language and compiler
Copyright (C) 2026 Egecan Akıncıoğlu

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published
by the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program.  If not, see <https://www.gnu.org/licenses/>.
*/

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

#[derive(Debug, Clone)]
struct VarSlot<'ctx> {
    ptr        : PointerValue<'ctx>,
    ty         : BasicTypeEnum<'ctx>,
    class_name : Option<String>,
    elem_class : Option<String>,
    enum_name  : Option<String>,
}

pub struct CodeGen<'ctx> {
    ctx     : &'ctx Context,
    module  : Module<'ctx>,
    builder : Builder<'ctx>,
    scopes  : Vec<HashMap<String, VarSlot<'ctx>>>,
    fns     : HashMap<String, FunctionValue<'ctx>>,
    cur_fn  : Option<FunctionValue<'ctx>>,
    struct_types  : HashMap<String, inkwell::types::StructType<'ctx>>,
    field_indices : HashMap<String, HashMap<String, u32>>,
    cur_class : Option<String>,
    default_params: HashMap<String, Vec<Option<Expr>>>,
    enum_variants : HashMap<String, HashMap<String, u32>>,
    static_fields : HashMap<String, inkwell::values::GlobalValue<'ctx>>,
    field_arimo_types  : HashMap<String, HashMap<String, String>>,
    field_elem_classes : HashMap<String, HashMap<String, String>>,
    fn_return_class    : HashMap<String, String>,
    param_class_map    : HashMap<String, String>,
    param_elem_map     : HashMap<String, String>,
    lambda_counter    : usize,
    loop_exit_bbs     : Vec<inkwell::basic_block::BasicBlock<'ctx>>,
    loop_continue_bbs : Vec<inkwell::basic_block::BasicBlock<'ctx>>,
    refcount_indices     : HashMap<String, u32>,
    manual_memory_classes: std::collections::HashSet<String>,
    finally_defers       : Vec<Vec<Stmt>>,
    defer_stack          : Vec<Vec<Expr>>,
    try_saved_tops       : Vec<inkwell::values::PointerValue<'ctx>>,
}

impl<'ctx> CodeGen<'ctx> {
    pub fn new(ctx: &'ctx Context, module_name: &str) -> Self {
        let module  = ctx.create_module(module_name);
        let builder = ctx.create_builder();
        CodeGen {
            ctx,
            module,
            builder,
            scopes        : Vec::new(),
            fns           : HashMap::new(),
            cur_fn        : None,
            struct_types  : HashMap::new(),
            field_indices : HashMap::new(),
            cur_class     : None,
            default_params: HashMap::new(),
            enum_variants : HashMap::new(),
            static_fields         : HashMap::new(),
            field_arimo_types     : HashMap::new(),
            field_elem_classes    : HashMap::new(),
            fn_return_class       : HashMap::new(),
            param_class_map       : HashMap::new(),
            param_elem_map        : HashMap::new(),
            lambda_counter        : 0,
            loop_exit_bbs         : Vec::new(),
            loop_continue_bbs     : Vec::new(),
            refcount_indices      : HashMap::new(),
            manual_memory_classes : std::collections::HashSet::new(),
            finally_defers        : Vec::new(),
            defer_stack           : Vec::new(),
            try_saved_tops        : Vec::new(),
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
        self.defer_stack.push(Vec::new());
    }

    fn arc_retain_ptr(
        &mut self,
        ptr        : inkwell::values::PointerValue<'ctx>,
        class_name : &str,
    ) -> CgResult<()> {
        if self.manual_memory_classes.contains(class_name) { return Ok(()); }
        if !self.refcount_indices.contains_key(class_name)  { return Ok(()); }
        if self.current_block_terminated()                   { return Ok(()); }
        let cur_fn = match self.cur_fn { Some(f) => f, None => return Ok(()) };

        let i64_ty  = self.ctx.i64_type();
        let inc_bb  = self.ctx.append_basic_block(cur_fn, "arc.inc");
        let cont_bb = self.ctx.append_basic_block(cur_fn, "arc.ri_cont");

        let pi      = self.builder.build_ptr_to_int(ptr, i64_ty, "arc_ri_pi")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let is_null = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ, pi, i64_ty.const_int(0, false), "arc_ri_null"
        ).map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(is_null, cont_bb, inc_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(inc_bb);
        let rc_idx    = *self.refcount_indices.get(class_name).unwrap();
        let struct_ty = *self.struct_types.get(class_name).unwrap();
        let rc_gep    = self.builder.build_struct_gep(struct_ty, ptr, rc_idx, "arc_ri_gep")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let rc_old    = self.builder.build_load(i64_ty, rc_gep, "arc_ri_rc")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let rc_new    = self.builder.build_int_add(rc_old, i64_ty.const_int(1, false), "arc_ri_inc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(rc_gep, rc_new)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(cont_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(cont_bb);
        Ok(())
    }

    fn infer_expr_class_name(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Ident(name) => self.lookup_var(name)
                .and_then(|s| s.class_name.clone())
                .filter(|cn| self.struct_types.contains_key(cn.as_str())),
            _ => None,
        }
    }

    fn arc_release_all_scopes_except(&mut self, skip_var: Option<&str>) -> CgResult<()> {
        for i in (0..self.scopes.len()).rev() {
            if self.current_block_terminated() { break; }
            let slots: Vec<(String, VarSlot<'ctx>)> = self.scopes[i]
                .iter()
                .filter(|(k, s)| {
                    s.class_name.is_some()
                        && skip_var.map_or(true, |skip| k.as_str() != skip)
                })
                .map(|(k, s)| (k.clone(), s.clone()))
                .collect();
            for (_, slot) in slots {
                if self.current_block_terminated() { break; }
                self.arc_release_var(slot)?;
            }
        }
        Ok(())
    }

    fn arc_release_var(&mut self, slot: VarSlot<'ctx>) -> CgResult<()> {
        let class_name = match &slot.class_name {
            Some(n) if !self.manual_memory_classes.contains(n.as_str())
                    && self.refcount_indices.contains_key(n.as_str()) => n.clone(),
            _ => return Ok(()),
        };
        if self.current_block_terminated() { return Ok(()); }
        let cur_fn = match self.cur_fn { Some(f) => f, None => return Ok(()) };

        let ptr_val = self.builder.build_load(slot.ty, slot.ptr, "arc_ptr")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let ptr = match ptr_val {
            BasicValueEnum::PointerValue(p) => p,
            _ => return Ok(()),
        };

        let i64_ty  = self.ctx.i64_type();
        let ptr_ty  = self.ctx.ptr_type(AddressSpace::default());
        let dec_bb  = self.ctx.append_basic_block(cur_fn, "arc.dec");
        let free_bb = self.ctx.append_basic_block(cur_fn, "arc.free");
        let cont_bb = self.ctx.append_basic_block(cur_fn, "arc.cont");

        let ptr_int  = self.builder.build_ptr_to_int(ptr, i64_ty, "arc_pi")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let is_null  = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ, ptr_int, i64_ty.const_int(0, false), "arc_null"
        ).map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(is_null, cont_bb, dec_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(dec_bb);
        let rc_idx    = *self.refcount_indices.get(&class_name).unwrap();
        let struct_ty = *self.struct_types.get(&class_name).unwrap();
        let rc_gep    = self.builder.build_struct_gep(struct_ty, ptr, rc_idx, "arc_rc_gep")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let rc_old    = self.builder.build_load(i64_ty, rc_gep, "arc_rc")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let rc_new    = self.builder.build_int_sub(rc_old, i64_ty.const_int(1, false), "arc_rc_dec")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(rc_gep, rc_new)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let is_zero   = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ, rc_new, i64_ty.const_int(0, false), "arc_zero"
        ).map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(is_zero, free_bb, cont_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(free_bb);
        let free_fn = self.module.get_function("free").unwrap_or_else(|| {
            let ft = self.ctx.void_type().fn_type(&[ptr_ty.into()], false);
            self.module.add_function("free", ft, None)
        });
        self.builder.build_call(free_fn, &[ptr.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(cont_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(cont_bb);
        Ok(())
    }

    fn arc_release_scope(&mut self, scope_idx: usize) -> CgResult<()> {
        let slots: Vec<VarSlot<'ctx>> = self.scopes[scope_idx]
            .values()
            .filter(|s| s.class_name.is_some())
            .cloned()
            .collect();
        for slot in slots {
            if self.current_block_terminated() { break; }
            self.arc_release_var(slot)?;
        }
        Ok(())
    }

    fn arc_release_all_scopes(&mut self) -> CgResult<()> {
        for i in (0..self.scopes.len()).rev() {
            if self.current_block_terminated() { break; }
            self.arc_release_scope(i)?;
        }
        Ok(())
    }

    fn pop_scope(&mut self) {
        if !self.current_block_terminated() {
            let defers = self.defer_stack.last().cloned().unwrap_or_default();
            for expr in defers.iter().rev() {
                if self.current_block_terminated() { break; }
                let _ = self.compile_expr(expr);
            }
        }
        self.defer_stack.pop();

        if let Some(idx) = self.scopes.len().checked_sub(1) {
            if !self.current_block_terminated() {
                let _ = self.arc_release_scope(idx);
            }
        }
        self.scopes.pop();
    }

    fn pop_scope_no_arc(&mut self) {
        self.defer_stack.pop();
        self.scopes.pop();
    }

    fn define_var(&mut self, name: &str, ptr: PointerValue<'ctx>, ty: BasicTypeEnum<'ctx>) {
        self.define_var_with_class(name, ptr, ty, None);
    }

    fn define_var_with_class(
        &mut self,
        name       : &str,
        ptr        : PointerValue<'ctx>,
        ty         : BasicTypeEnum<'ctx>,
        class_name : Option<String>,
    ) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), VarSlot { ptr, ty, class_name, elem_class: None, enum_name: None });
        }
    }

    fn define_enum_var(&mut self, name: &str, ptr: PointerValue<'ctx>, ty: BasicTypeEnum<'ctx>, enum_name: String) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), VarSlot { ptr, ty, class_name: None, elem_class: None, enum_name: Some(enum_name) });
        }
    }

    fn define_collection_var(
        &mut self,
        name       : &str,
        ptr        : PointerValue<'ctx>,
        ty         : BasicTypeEnum<'ctx>,
        class_name : Option<String>,
        elem_class : Option<String>,
    ) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), VarSlot { ptr, ty, class_name, elem_class, enum_name: None });
        }
    }

    fn infer_elem_class(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Ident(name) => {
                for scope in self.scopes.iter().rev() {
                    if let Some(slot) = scope.get(name.as_str()) {
                        if slot.elem_class.is_some() {
                            return slot.elem_class.clone();
                        }
                    }
                }
                None
            }
            _ => None,
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
            Type::List(_) | Type::Map(..) | Type::HashMap(..) | Type::TreeMap(..)
            | Type::Pair(..) | Type::Slice(_) | Type::Array(..)
                => Some(self.ctx.ptr_type(AddressSpace::default()).into()),
            Type::Generic(_, _)   => Some(self.ctx.ptr_type(AddressSpace::default()).into()),
            Type::Nullable(inner) => self.llvm_type(inner),
            Type::Named(n) if self.is_enum(n) => Some(self.ctx.i32_type().into()),
            Type::Named(n) if n == "Vec4f" => Some(self.ctx.f32_type().vec_type(4).into()),
            Type::Named(n) if n == "Vec8f" => Some(self.ctx.f32_type().vec_type(8).into()),
            Type::Named(n) if n == "Vec4i" => Some(self.ctx.i32_type().vec_type(4).into()),
            Type::Named(n) if n == "Vec8i" => Some(self.ctx.i32_type().vec_type(8).into()),
            Type::Named(_) => Some(self.ctx.ptr_type(AddressSpace::default()).into()),
            Type::FnPtr(_, _) => {
                let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                Some(self.ctx.struct_type(&[ptr_ty.into(), ptr_ty.into()], false).into())
            }
            _ => Some(self.ctx.ptr_type(AddressSpace::default()).into()),
        }
    }

    pub fn compile_module(&mut self, module: &crate::ast::Module) -> CgResult<()> {
        for item in &module.items {
            if let Item::Extern(ext) = item {
                self.declare_extern_block(ext)?;
            }
        }

        self.declare_printf();
        self.declare_malloc();
        self.declare_collection_runtime();

        for item in &module.items {
            if let Item::Class(c) = item {
                if c.manual {
                    self.manual_memory_classes.insert(c.name.clone());
                }
            }
        }

        for item in &module.items {
            if let Item::Enum(e) = item {
                self.register_enum(e);
            }
        }
        for item in &module.items {
            match item {
                Item::Class(c) => self.register_class_struct(c),
                Item::Struct(s) => self.register_struct_decl(s),
                _ => {}
            }
        }
        for item in &module.items {
            if let Item::Class(c) = item {
                self.register_class_methods(c)?;
            }
        }
        for item in &module.items {
            if let Item::Class(c) = item {
                self.init_static_fields(c)?;
            }
        }
        for item in &module.items {
            if let Item::Extension(ext) = item {
                self.register_extension_methods(ext)?;
            }
        }
        for item in &module.items {
            match item {
                Item::Class(c)     => self.compile_class(c)?,
                Item::Enum(e)      => self.compile_enum(e)?,
                Item::Extension(e) => self.compile_extension(e)?,
                _                  => {}
            }
        }
        Ok(())
    }

    fn register_extension_methods(&mut self, ext: &ExtensionDecl) -> CgResult<()> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let target_llvm = match ext.target.as_str() {
            "Integer" | "Long"   => self.ctx.i64_type().as_basic_type_enum(),
            "Float"   | "Double" => self.ctx.f64_type().as_basic_type_enum(),
            "Boolean"            => self.ctx.bool_type().as_basic_type_enum(),
            "String"             => ptr_ty.as_basic_type_enum(),
            _                    => ptr_ty.as_basic_type_enum(),
        };
        for m in &ext.methods {
            let fn_name = format!("{}_{}", ext.target, m.name);
            if self.module.get_function(&fn_name).is_some() { continue; }
            let mut param_types: Vec<inkwell::types::BasicMetadataTypeEnum> = vec![target_llvm.into()];
            for p in &m.params {
                if let Some(t) = self.llvm_type(&p.ty) { param_types.push(t.into()); }
            }
            let fn_val = match &m.return_ty {
                Some(rt) => match self.llvm_type(rt) {
                    Some(rty) => self.module.add_function(&fn_name, rty.fn_type(&param_types, false), None),
                    None      => self.module.add_function(&fn_name, self.ctx.void_type().fn_type(&param_types, false), None),
                },
                None => self.module.add_function(&fn_name, self.ctx.void_type().fn_type(&param_types, false), None),
            };
            self.fns.insert(fn_name, fn_val);
        }
        Ok(())
    }

    fn compile_extension(&mut self, ext: &ExtensionDecl) -> CgResult<()> {
        let target_llvm = match ext.target.as_str() {
            "Integer" | "Long"   => self.ctx.i64_type().as_basic_type_enum(),
            "Float"   | "Double" => self.ctx.f64_type().as_basic_type_enum(),
            "Boolean"            => self.ctx.bool_type().as_basic_type_enum(),
            "String"             => self.ctx.ptr_type(AddressSpace::default()).as_basic_type_enum(),
            _                    => self.ctx.ptr_type(AddressSpace::default()).as_basic_type_enum(),
        };
        for m in &ext.methods {
            if m.body.is_none() { continue; }
            let fn_name = format!("{}_{}", ext.target, m.name);
            let fn_val = match self.fns.get(&fn_name).copied()
                .or_else(|| self.module.get_function(&fn_name)) {
                Some(f) => f,
                None    => continue,
            };
            let entry = self.ctx.append_basic_block(fn_val, "entry");
            self.builder.position_at_end(entry);
            self.cur_fn = Some(fn_val);
            self.push_scope();

            if let Some(this_val) = fn_val.get_nth_param(0) {
                let alloca = self.builder.build_alloca(target_llvm, "this")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_store(alloca, this_val)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.define_var("this", alloca, target_llvm);
            }

            for (i, p) in m.params.iter().enumerate() {
                if let Some(llvm_ty) = self.llvm_type(&p.ty) {
                    if let Some(pv) = fn_val.get_nth_param((i + 1) as u32) {
                        let alloca = self.builder.build_alloca(llvm_ty, &p.name)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        self.builder.build_store(alloca, pv)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        self.define_var(&p.name, alloca, llvm_ty);
                    }
                }
            }

            let body = m.body.as_ref().unwrap();
            let mut returned = false;
            for stmt in body {
                if self.compile_stmt(stmt)? { returned = true; break; }
            }
            if !returned && !self.current_block_terminated() {
                match fn_val.get_type().get_return_type() {
                    None => { self.builder.build_return(None).map_err(|e| CodeGenError::new(e.to_string()))?; }
                    Some(_) => {
                        let zero = self.ctx.i64_type().const_int(0, false);
                        self.builder.build_return(Some(&zero)).map_err(|e| CodeGenError::new(e.to_string()))?;
                    }
                }
            }
            self.pop_scope();
            self.cur_fn = None;
            fn_val.verify(true);
        }
        Ok(())
    }

    fn compile_asm(&mut self, code: &str) -> CgResult<()> {
        let void_ty = self.ctx.void_type();
        let fn_ty   = void_ty.fn_type(&[], false);
        let asm_code = code.replace("\\n", "\n").replace("\\t", "\t");
        let inline_asm = self.ctx.create_inline_asm(
            fn_ty,
            asm_code,
            String::new(),
            true,
            false,
            None,
            false,
        );
        self.builder.build_indirect_call(fn_ty, inline_asm, &[], "asm_call")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        Ok(())
    }

    fn declare_malloc(&mut self) {
        if self.module.get_function("malloc").is_some() { return; }
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64_ty = self.ctx.i64_type();
        let malloc_ty = ptr_ty.fn_type(&[i64_ty.into()], false);
        self.module.add_function("malloc", malloc_ty, None);
    }

    fn declare_setjmp(&mut self) {
        #[cfg(target_os = "windows")]
        let fn_name = "_setjmp";
        #[cfg(not(target_os = "windows"))]
        let fn_name = "setjmp";

        if self.module.get_function(fn_name).is_some() { return; }
        let i32_ty = self.ctx.i32_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        #[cfg(target_os = "windows")]
        let ft = i32_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
        #[cfg(not(target_os = "windows"))]
        let ft = i32_ty.fn_type(&[ptr_ty.into()], false);
        let f = self.module.add_function(fn_name, ft, None);
        let kind_id = inkwell::attributes::Attribute::get_named_enum_kind_id("returns_twice");
        if kind_id != 0 {
            let attr = self.ctx.create_enum_attribute(kind_id, 0);
            f.add_attribute(inkwell::attributes::AttributeLoc::Function, attr);
        }
    }

    fn setjmp_fn_name(&self) -> &'static str {
        #[cfg(target_os = "windows")] { "_setjmp" }
        #[cfg(not(target_os = "windows"))] { "setjmp" }
    }

    fn declare_longjmp(&mut self) {
        if self.module.get_function("longjmp").is_some() { return; }
        let void_ty = self.ctx.void_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i32_ty = self.ctx.i32_type();
        let ft = void_ty.fn_type(&[ptr_ty.into(), i32_ty.into()], false);
        let f = self.module.add_function("longjmp", ft, None);
        let kind_id = inkwell::attributes::Attribute::get_named_enum_kind_id("noreturn");
        if kind_id != 0 {
            let attr = self.ctx.create_enum_attribute(kind_id, 0);
            f.add_attribute(inkwell::attributes::AttributeLoc::Function, attr);
        }
    }

    fn declare_strcmp(&mut self) {
        if self.module.get_function("strcmp").is_some() { return; }
        let i32_ty = self.ctx.i32_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let ft = i32_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
        self.module.add_function("strcmp", ft, None);
    }

    fn get_or_create_env_global(&mut self, name: &str, ty: BasicTypeEnum<'ctx>) -> inkwell::values::GlobalValue<'ctx> {
        if let Some(g) = self.module.get_global(name) { return g; }
        let zero = self.make_zero_value(ty);
        let g = self.module.add_global(ty, None, name);
        g.set_initializer(&zero);
        g.set_linkage(inkwell::module::Linkage::Internal);
        g
    }

    fn get_or_create_eh_global(&mut self, name: &str) -> inkwell::values::GlobalValue<'ctx> {
        if let Some(g) = self.module.get_global(name) { return g; }
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let g = self.module.add_global(ptr_ty, None, name);
        g.set_initializer(&ptr_ty.const_null());
        g.set_linkage(inkwell::module::Linkage::Internal);
        g
    }

    fn get_or_create_eh_jmpbufs(&mut self) -> inkwell::values::GlobalValue<'ctx> {
        if let Some(g) = self.module.get_global("__arimo_ex_jmpbufs") { return g; }
        let i64_ty = self.ctx.i64_type();
        let jmpbuf_ty = i64_ty.array_type(32);
        let arr_ty = jmpbuf_ty.array_type(32);
        let g = self.module.add_global(arr_ty, None, "__arimo_ex_jmpbufs");
        g.set_initializer(&arr_ty.const_zero());
        g.set_linkage(inkwell::module::Linkage::Internal);
        g.set_alignment(32);
        g
    }

    fn get_or_create_eh_depth(&mut self) -> inkwell::values::GlobalValue<'ctx> {
        if let Some(g) = self.module.get_global("__arimo_ex_depth") { return g; }
        let i32_ty = self.ctx.i32_type();
        let g = self.module.add_global(i32_ty, None, "__arimo_ex_depth");
        g.set_initializer(&i32_ty.const_int(0, false));
        g.set_linkage(inkwell::module::Linkage::Internal);
        g
    }

    fn get_jmpbuf_ptr(&mut self, slot: inkwell::values::IntValue<'ctx>) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        let i64_ty = self.ctx.i64_type();
        let jmpbuf_slot_ty = i64_ty.array_type(32);
        let arr_ty = jmpbuf_slot_ty.array_type(32);
        let jmpbufs_gv = self.get_or_create_eh_jmpbufs();
        let i32_ty = self.ctx.i32_type();
        let zero = i32_ty.const_int(0, false);
        let ptr = unsafe {
            self.builder.build_gep(arr_ty, jmpbufs_gv.as_pointer_value(), &[zero, slot], "jmpbuf_slot")
        }.map_err(|e| CodeGenError::new(e.to_string()))?;
        Ok(ptr)
    }

    fn register_class_struct(&mut self, c: &ClassDecl) {
        let mut all_field_types: Vec<BasicTypeEnum<'ctx>> = Vec::new();
        let mut idx_map: HashMap<String, u32> = HashMap::new();
        let mut idx = 0u32;

        if let Some(parent_name) = &c.extends {
            if let Some(parent_fields) = self.field_indices.get(parent_name).cloned() {
                if let Some(parent_struct) = self.struct_types.get(parent_name).copied() {
                    for fi in 0..parent_struct.count_fields() {
                        if let Some(ft) = parent_struct.get_field_type_at_index(fi) {
                            all_field_types.push(ft);
                        }
                    }
                    for (fname, fidx) in &parent_fields {
                        idx_map.insert(fname.clone(), *fidx);
                    }
                    idx = parent_struct.count_fields() as u32;
                }
            }
        }

        for f in c.fields.iter().filter(|f| !f.static_) {
            if let Some(ft) = self.llvm_type(&f.ty) {
                all_field_types.push(ft);
                idx_map.insert(f.name.clone(), idx);
                idx += 1;
            }
        }

        let rc_idx = if let Some(parent) = &c.extends {
            self.refcount_indices.get(parent.as_str()).copied().unwrap_or_else(|| {
                let i = all_field_types.len() as u32;
                all_field_types.push(self.ctx.i64_type().into());
                i
            })
        } else {
            let i = all_field_types.len() as u32;
            all_field_types.push(self.ctx.i64_type().into());
            i
        };
        self.refcount_indices.insert(c.name.clone(), rc_idx);

        let struct_ty = self.ctx.struct_type(&all_field_types, false);
        self.struct_types.insert(c.name.clone(), struct_ty);
        self.field_indices.insert(c.name.clone(), idx_map);

        let mut arimo_types: HashMap<String, String> = HashMap::new();
        if let Some(parent_name) = &c.extends {
            if let Some(parent_arimo) = self.field_arimo_types.get(parent_name).cloned() {
                arimo_types.extend(parent_arimo);
            }
        }
        for f in c.fields.iter().filter(|f| !f.static_) {
            let type_name = match &f.ty {
                Type::Named(n) => n.clone(),
                Type::List(_) => "__List".to_string(),
                Type::HashMap(..) | Type::Map(..) | Type::TreeMap(..) => "__HashMap".to_string(),
                Type::Pair(..) => "__Pair".to_string(),
                Type::Nullable(inner) => match inner.as_ref() {
                    Type::Named(n) => n.clone(),
                    Type::List(_) => "__List".to_string(),
                    Type::HashMap(..) | Type::Map(..) | Type::TreeMap(..) => "__HashMap".to_string(),
                    other => format!("{:?}", other),
                },
                other => format!("{:?}", other),
            };
            arimo_types.insert(f.name.clone(), type_name);
        }
        self.field_arimo_types.insert(c.name.clone(), arimo_types);

        let mut elem_classes: HashMap<String, String> = HashMap::new();
        if let Some(parent_name) = &c.extends {
            if let Some(parent_ec) = self.field_elem_classes.get(parent_name).cloned() {
                elem_classes.extend(parent_ec);
            }
        }
        for f in c.fields.iter().filter(|f| !f.static_) {
            if let Type::List(inner) = &f.ty {
                let ec = match inner.as_ref() {
                    Type::Named(n)  => Some(n.clone()),
                    Type::Str       => Some("String".to_string()),
                    Type::Integer   => Some("Integer".to_string()),
                    Type::Float     => Some("Float".to_string()),
                    Type::Boolean   => Some("Boolean".to_string()),
                    _ => None,
                };
                if let Some(ec) = ec { elem_classes.insert(f.name.clone(), ec); }
            }
        }
        self.field_elem_classes.insert(c.name.clone(), elem_classes);

        for f in c.fields.iter().filter(|f| f.static_) {
            let global_name = format!("{}_{}", c.name, f.name);
            if self.module.get_global(&global_name).is_some() { continue; }

            if let Some(llvm_ty) = self.llvm_type(&f.ty) {
                let zero = self.make_zero_value(llvm_ty);
                let global = self.module.add_global(llvm_ty, None, &global_name);
                global.set_initializer(&zero);
                global.set_linkage(inkwell::module::Linkage::Internal);
                self.static_fields.insert(global_name, global);
            }
        }
    }

    fn register_struct_decl(&mut self, s: &StructDecl) {
        let mut field_types: Vec<BasicTypeEnum<'ctx>> = Vec::new();
        let mut idx_map: HashMap<String, u32> = HashMap::new();

        for (i, f) in s.fields.iter().enumerate() {
            if let Some(ft) = self.llvm_type(&f.ty) {
                field_types.push(ft);
                idx_map.insert(f.name.clone(), i as u32);
            }
        }

        let struct_ty = self.ctx.struct_type(&field_types, s.packed);
        self.struct_types.insert(s.name.clone(), struct_ty);
        self.field_indices.insert(s.name.clone(), idx_map);
    }

    fn make_zero_value(&self, ty: BasicTypeEnum<'ctx>) -> inkwell::values::BasicValueEnum<'ctx> {
        match ty {
            BasicTypeEnum::IntType(t)     => t.const_int(0, false).into(),
            BasicTypeEnum::FloatType(t)   => t.const_float(0.0).into(),
            BasicTypeEnum::PointerType(t) => t.const_null().into(),
            _                             => self.ctx.i64_type().const_int(0, false).into(),
        }
    }

    fn init_static_fields(&mut self, c: &ClassDecl) -> CgResult<()> {
        for f in c.fields.iter().filter(|f| f.static_) {
            let global_name = format!("{}_{}", c.name, f.name);
            if let Some(gv) = self.module.get_global(&global_name) {
                if let Some(init_expr) = &f.value {
                    let const_val: Option<inkwell::values::BasicValueEnum> = match init_expr {
                        Expr::IntLit(n)   => Some(self.ctx.i64_type().const_int(*n as u64, *n < 0).into()),
                        Expr::FloatLit(f) => Some(self.ctx.f64_type().const_float(*f).into()),
                        Expr::BoolLit(b)  => Some(self.ctx.bool_type().const_int(*b as u64, false).into()),
                        Expr::StrLit(s)   => {
                            let bytes = s.as_bytes();
                            let char_arr = self.ctx.const_string(bytes, true);
                            let str_global = self.module.add_global(
                                char_arr.get_type(), None,
                                &format!("{}_strdata", global_name)
                            );
                            str_global.set_initializer(&char_arr);
                            str_global.set_linkage(inkwell::module::Linkage::Internal);
                            Some(str_global.as_pointer_value().into())
                        }
                        _ => None,
                    };
                    if let Some(cv) = const_val {
                        gv.set_initializer(&cv);
                    }
                }
            }
        }
        Ok(())
    }

    fn register_enum(&mut self, e: &EnumDecl) {
        let mut variants = HashMap::new();
        for (i, v) in e.variants.iter().enumerate() {
            variants.insert(v.name.clone(), i as u32);
        }
        self.enum_variants.insert(e.name.clone(), variants);

        let i32_ty = self.ctx.i32_type();
        for m in &e.methods {
            let fn_name = format!("{}_{}", e.name, m.name);
            if self.module.get_function(&fn_name).is_some() { continue; }

            let mut param_types: Vec<inkwell::types::BasicMetadataTypeEnum> =
                vec![i32_ty.into()];
            for p in &m.params {
                if let Some(t) = self.llvm_type(&p.ty) {
                    param_types.push(t.into());
                }
            }

            let fn_val = match &m.return_ty {
                Some(ret_ty) => match self.llvm_type(ret_ty) {
                    Some(rt) => self.module.add_function(&fn_name, rt.fn_type(&param_types, false), None),
                    None     => self.module.add_function(&fn_name, self.ctx.void_type().fn_type(&param_types, false), None),
                },
                None => self.module.add_function(&fn_name, self.ctx.void_type().fn_type(&param_types, false), None),
            };
            self.fns.insert(fn_name, fn_val);
        }
    }

    fn is_enum(&self, name: &str) -> bool {
        self.enum_variants.contains_key(name)
    }

    fn enum_variant_value(&self, enum_name: &str, variant: &str) -> Option<u32> {
        self.enum_variants.get(enum_name)?.get(variant).copied()
    }

    fn declare_printf(&mut self) {
        if self.module.get_function("printf").is_some() { return; }
        let i8_ptr = self.ctx.ptr_type(AddressSpace::default());
        let i32_ty = self.ctx.i32_type();
        let printf_ty = i32_ty.fn_type(&[i8_ptr.into()], true);
        self.module.add_function("printf", printf_ty, None);
    }

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

    fn register_class_methods(&mut self, c: &ClassDecl) -> CgResult<()> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());

        if c.constructor.is_some() {
            let ctor_name = format!("{}_new", c.name);
            if self.module.get_function(&ctor_name).is_none() {
                let ctor = c.constructor.as_ref().unwrap();
                let params: Vec<inkwell::types::BasicMetadataTypeEnum> = ctor.params.iter()
                    .filter_map(|p| self.llvm_type(&p.ty).map(|t| t.into()))
                    .collect();
                let fn_ty = ptr_ty.fn_type(&params, false);
                let fn_val = self.module.add_function(&ctor_name, fn_ty, None);
                self.fns.insert(ctor_name, fn_val);
            }
        }

        for m in &c.methods {
            let fn_name = format!("{}_{}", c.name, m.name);
            if self.module.get_function(&fn_name).is_some() { continue; }

            let mut param_types: Vec<inkwell::types::BasicMetadataTypeEnum> = Vec::new();
            if !m.static_ {
                param_types.push(ptr_ty.into());
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
            self.fns.insert(fn_name.clone(), fn_val);

            if let Some(Type::Named(ret_class)) = &m.return_ty {
                self.fn_return_class.insert(fn_name.clone(), ret_class.clone());
            }

            // Store default param expressions
            let defaults: Vec<Option<Expr>> = m.params.iter()
                .map(|p| p.default.clone())
                .collect();
            if defaults.iter().any(|d| d.is_some()) {
                self.default_params.insert(fn_name, defaults);
            }
        }
        Ok(())
    }

    fn compile_enum(&mut self, e: &EnumDecl) -> CgResult<()> {
        let variant_names: Vec<String> = e.variants.iter().map(|v| v.name.clone()).collect();
        self.generate_enum_label_fn(&e.name.clone(), &variant_names);
        self.cur_class = Some(e.name.clone());
        for m in &e.methods.clone() {
            if m.body.is_none() { continue; }
            self.compile_enum_method(&e.name.clone(), m)?;
        }
        self.cur_class = None;
        Ok(())
    }

    fn generate_enum_label_fn(&mut self, enum_name: &str, variant_names: &[String]) {
        let fn_name = format!("{}_label", enum_name);
        if self.module.get_function(&fn_name).is_some() { return; }
        let prev_block = self.builder.get_insert_block();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i32_ty = self.ctx.i32_type();
        let ft = ptr_ty.fn_type(&[i32_ty.into()], false);
        let fn_val = self.module.add_function(&fn_name, ft, None);
        let entry_bb  = self.ctx.append_basic_block(fn_val, "entry");
        let default_bb = self.ctx.append_basic_block(fn_val, "sw.default");
        let end_bb    = self.ctx.append_basic_block(fn_val, "sw.end");
        self.builder.position_at_end(entry_bb);
        let param = fn_val.get_first_param().unwrap().into_int_value();
        let mut arm_bbs: Vec<inkwell::basic_block::BasicBlock<'ctx>> = Vec::new();
        let mut switch_cases: Vec<(inkwell::values::IntValue<'ctx>, inkwell::basic_block::BasicBlock<'ctx>)> = Vec::new();
        for i in 0..variant_names.len() {
            let arm_bb = self.ctx.append_basic_block(fn_val, &format!("sw.{}", i));
            arm_bbs.push(arm_bb);
            switch_cases.push((i32_ty.const_int(i as u64, false), arm_bb));
        }
        self.builder.build_switch(param, default_bb, &switch_cases).unwrap();
        let mut arm_str_ptrs: Vec<PointerValue<'ctx>> = Vec::new();
        for (i, vname) in variant_names.iter().enumerate() {
            self.builder.position_at_end(arm_bbs[i]);
            let sp = self.build_global_string(vname).unwrap();
            arm_str_ptrs.push(sp);
            self.builder.build_unconditional_branch(end_bb).unwrap();
        }
        self.builder.position_at_end(default_bb);
        let unknown_ptr = self.build_global_string("<unknown>").unwrap();
        self.builder.build_unconditional_branch(end_bb).unwrap();
        self.builder.position_at_end(end_bb);
        let phi = self.builder.build_phi(ptr_ty, "lbl").unwrap();
        for (i, sp) in arm_str_ptrs.iter().enumerate() {
            phi.add_incoming(&[(sp, arm_bbs[i])]);
        }
        phi.add_incoming(&[(&unknown_ptr, default_bb)]);
        self.builder.build_return(Some(&phi.as_basic_value())).unwrap();
        if let Some(bb) = prev_block { self.builder.position_at_end(bb); }
    }

    fn try_enum_label(&mut self, expr: &Expr) -> Option<PointerValue<'ctx>> {
        let vname = match expr { Expr::Ident(n) => n.clone(), _ => return None };
        let slot = self.lookup_var(&vname).cloned()?;
        let en = slot.enum_name.clone()?;
        let label_fn = self.module.get_function(&format!("{}_label", en))?;
        let val = self.builder.build_load(slot.ty, slot.ptr, "enum_load_lbl").ok()?;
        let res = self.builder.build_call(label_fn, &[val.into()], "enum_lbl").ok()?;
        match res.try_as_basic_value().basic() {
            Some(BasicValueEnum::PointerValue(p)) => Some(p),
            _ => None,
        }
    }

    fn compile_enum_method(&mut self, enum_name: &str, m: &Method) -> CgResult<()> {
        let fn_name = format!("{}_{}", enum_name, m.name);
        let fn_val = match self.fns.get(&fn_name).copied()
            .or_else(|| self.module.get_function(&fn_name))
        {
            Some(f) => f,
            None    => return Ok(()),
        };

        let body = match m.body.as_ref() { Some(b) => b.clone(), None => return Ok(()) };

        let entry = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);
        self.cur_fn = Some(fn_val);
        self.push_scope();

        if let Some(this_val) = fn_val.get_nth_param(0) {
            let i32_ty = self.ctx.i32_type();
            let this_alloca = self.builder.build_alloca(i32_ty, "this")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            self.builder.build_store(this_alloca, this_val)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            self.define_var("this", this_alloca, i32_ty.into());
        }

        for (i, p) in m.params.iter().enumerate() {
            if let Some(llvm_ty) = self.llvm_type(&p.ty) {
                if let Some(pv) = fn_val.get_nth_param((i + 1) as u32) {
                    let alloca = self.builder.build_alloca(llvm_ty, &p.name)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_store(alloca, pv)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.define_var(&p.name, alloca, llvm_ty);
                }
            }
        }

        let mut returned = false;
        for stmt in &body {
            if self.compile_stmt(stmt)? { returned = true; break; }
        }
        if !returned {
            self.builder.build_return(None)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }

        self.pop_scope();
        self.cur_fn = None;

        if fn_val.verify(true) { Ok(()) }
        else { Err(CodeGenError::new(format!("enum method verify failed: {}_{}", enum_name, m.name))) }
    }

    fn compile_class(&mut self, c: &ClassDecl) -> CgResult<()> {
        self.cur_class = Some(c.name.clone());

        if let Some(ctor) = &c.constructor.clone() {
            self.compile_constructor(c, ctor)?;
        }

        for m in &c.methods.clone() {
            if m.body.is_none() { continue; }
            self.compile_method(c, m)?;
        }

        self.cur_class = None;
        Ok(())
    }

    fn compile_constructor(&mut self, c: &ClassDecl, ctor: &Constructor) -> CgResult<()> {
        let ctor_name = format!("{}_new", c.name);
        let fn_val = match self.fns.get(&ctor_name).copied()
            .or_else(|| self.module.get_function(&ctor_name))
        {
            Some(f) => f,
            None    => return Ok(()),
        };

        let entry = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);
        self.cur_fn = Some(fn_val);
        self.push_scope();

        let struct_ty = self.struct_types.get(&c.name).copied()
            .ok_or_else(|| CodeGenError::new(format!("struct type not found: {}", c.name)))?;
        let size = struct_ty.size_of()
            .ok_or_else(|| CodeGenError::new("struct has no size"))?;
        // Use calloc to zero-initialize: prevents ARC release of garbage field values
        let calloc_fn = if let Some(f) = self.module.get_function("calloc") { f } else {
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let i64_ty = self.ctx.i64_type();
            self.module.add_function("calloc", ptr_ty.fn_type(&[i64_ty.into(), i64_ty.into()], false), None)
        };
        let obj_ptr = self.builder
            .build_call(calloc_fn, &[self.ctx.i64_type().const_int(1, false).into(), size.into()], "obj")
            .map_err(|e| CodeGenError::new(e.to_string()))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| CodeGenError::new("calloc returned void"))?;

        if !self.manual_memory_classes.contains(&c.name) {
            if let Some(&rc_idx) = self.refcount_indices.get(&c.name) {
                if let BasicValueEnum::PointerValue(ptr) = obj_ptr {
                    let rc_gep = self.builder.build_struct_gep(struct_ty, ptr, rc_idx, "rc_init_gep")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_store(rc_gep, self.ctx.i64_type().const_int(1, false))
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                }
            }
        }

        let this_alloca = self.builder
            .build_alloca(self.ctx.ptr_type(AddressSpace::default()), "this")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(this_alloca, obj_ptr)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.define_var("this", this_alloca,
            self.ctx.ptr_type(AddressSpace::default()).into());

        for (i, p) in ctor.params.iter().enumerate() {
            if let Some(llvm_ty) = self.llvm_type(&p.ty) {
                if let Some(pv) = fn_val.get_nth_param(i as u32) {
                    let alloca = self.builder.build_alloca(llvm_ty, &p.name)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_store(alloca, pv)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.define_var(&p.name, alloca, llvm_ty);
                }
            }
        }

        for stmt in &ctor.body.clone() {
            if self.compile_stmt(stmt)? { break; }
        }

        let ptr_ty: BasicTypeEnum<'ctx> = self.ctx.ptr_type(AddressSpace::default()).into();
        let this_ptr = self.builder
            .build_load(ptr_ty, this_alloca, "ret_this")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_return(Some(&this_ptr))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.pop_scope();
        self.cur_fn = None;

        if fn_val.verify(true) { Ok(()) }
        else { Err(CodeGenError::new(format!("constructor verification failed: {}", c.name))) }
    }

    fn compile_method(&mut self, c: &ClassDecl, m: &Method) -> CgResult<()> {
        let fn_name = format!("{}_{}", c.name, m.name);

        let is_entry = m.name == "main" && m.static_;

        let fn_val = if is_entry {
            let i32_ty = self.ctx.i32_type();
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let main_ty = i32_ty.fn_type(&[i32_ty.into(), ptr_ty.into()], false);
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

        if m.return_ty.as_ref().map(|t| matches!(t, Type::NoReturn)).unwrap_or(false) {
            let kind_id = inkwell::attributes::Attribute::get_named_enum_kind_id("noreturn");
            if kind_id != 0 {
                let attr = self.ctx.create_enum_attribute(kind_id, 0);
                fn_val.add_attribute(inkwell::attributes::AttributeLoc::Function, attr);
            }
        }
        if m.inline_ {
            let kind_id = inkwell::attributes::Attribute::get_named_enum_kind_id("alwaysinline");
            if kind_id != 0 {
                let attr = self.ctx.create_enum_attribute(kind_id, 0);
                fn_val.add_attribute(inkwell::attributes::AttributeLoc::Function, attr);
            }
        }
        if m.pure_ {
            let kind_id = inkwell::attributes::Attribute::get_named_enum_kind_id("readnone");
            if kind_id != 0 {
                let attr = self.ctx.create_enum_attribute(kind_id, 0);
                fn_val.add_attribute(inkwell::attributes::AttributeLoc::Function, attr);
            }
        }
        if let Some(section) = &m.section {
            fn_val.set_section(Some(section.as_str()));
        }
        if let Some(cc) = &m.calling_conv {
            let llvm_cc = match cc {
                CallingConv::Cdecl    => inkwell::llvm_sys::LLVMCallConv::LLVMCCallConv as u32,
                CallingConv::Stdcall  => 64u32,
                CallingConv::Interrupt => 86u32,
            };
            fn_val.set_call_conventions(llvm_cc);
        }

        let entry_block = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry_block);

        self.cur_fn = Some(fn_val);
        self.try_saved_tops.clear();
        self.param_class_map.clear();
        self.param_elem_map.clear();
        self.push_scope();

        if is_entry {
            let i32_ty = self.ctx.i32_type();
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let argc_gv = self.get_or_create_env_global("__arimo_argc", i32_ty.into());
            let argv_gv = self.get_or_create_env_global("__arimo_argv", ptr_ty.into());
            if let (Some(argc_p), Some(argv_p)) = (fn_val.get_nth_param(0), fn_val.get_nth_param(1)) {
                let _ = self.builder.build_store(argc_gv.as_pointer_value(), argc_p);
                let _ = self.builder.build_store(argv_gv.as_pointer_value(), argv_p);
            }
        }

        if !m.static_ {
            if let Some(this_val) = fn_val.get_nth_param(0) {
                let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                let this_alloca = self.builder.build_alloca(ptr_ty, "this")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_store(this_alloca, this_val)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.define_var("this", this_alloca, ptr_ty.into());
            }
        }

        let param_offset = if m.static_ { 0 } else { 1 };
        for (i, p) in m.params.iter().enumerate() {
            if let Some(llvm_ty) = self.llvm_type(&p.ty) {
                let param_val = fn_val.get_nth_param((i + param_offset) as u32);
                if let Some(pv) = param_val {
                    let alloca = self.builder.build_alloca(llvm_ty, &p.name)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_store(alloca, pv)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let (class_name, elem_class) = match &p.ty {
                        Type::Named(n) => (Some(n.clone()), None),
                        Type::Nullable(inner) => match inner.as_ref() {
                            Type::Named(n) => (Some(n.clone()), None),
                            Type::List(li) => {
                                let ec = match li.as_ref() {
                                    Type::Named(n)  => Some(n.clone()),
                                    Type::Str       => Some("String".to_string()),
                                    _ => None,
                                };
                                (Some("__List".to_string()), ec)
                            }
                            _ => (None, None),
                        },
                        Type::List(inner) => {
                            let ec = match inner.as_ref() {
                                Type::Named(n)  => Some(n.clone()),
                                Type::Str       => Some("String".to_string()),
                                Type::Integer   => Some("Integer".to_string()),
                                Type::Float     => Some("Float".to_string()),
                                Type::Boolean   => Some("Boolean".to_string()),
                                _ => None,
                            };
                            (Some("__List".to_string()), ec)
                        }
                        Type::HashMap(..) | Type::Map(..) | Type::TreeMap(..) =>
                            (Some("__HashMap".to_string()), None),
                        Type::Pair(..) => (Some("__Pair".to_string()), None),
                        _ => (None, None),
                    };
                    // Always define_var (no ARC for params), but store type info separately
                    self.define_var(&p.name, alloca, llvm_ty);
                    if let Some(cn) = class_name {
                        self.param_class_map.insert(p.name.clone(), cn);
                    }
                    if let Some(ec) = elem_class {
                        self.param_elem_map.insert(p.name.clone(), ec);
                    }
                }
            }
        }

        let mut returned = false;
        for stmt in body {
            if self.compile_stmt(stmt)? {
                returned = true;
                break;
            }
        }

        if !returned {
            if !self.current_block_terminated() {
                if let Some(idx) = self.scopes.len().checked_sub(1) {
                    self.arc_release_scope(idx)?;
                }
            }
            if is_entry {
                let zero = self.ctx.i32_type().const_int(0, false);
                self.builder.build_return(Some(&zero))
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
            } else {
                self.builder.build_return(None)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
            }
        }

        self.pop_scope_no_arc();
        self.cur_fn = None;

        if fn_val.verify(true) {
            Ok(())
        } else {
            Err(CodeGenError::new(format!("LLVM function verification failed: {}", fn_name)))
        }
    }

    fn compile_stmt(&mut self, stmt: &Stmt) -> CgResult<bool> {
        match stmt {
            Stmt::Return(expr) => {
                let (ret_val, skip_var_name) = match expr {
                    None => (None, None),
                    Some(e) => {
                        let skip = match e {
                            Expr::Ident(n) => Some(n.clone()),
                            _ => None,
                        };
                        let val  = self.compile_expr(e)?;
                        (val, skip)
                    }
                };

                let defers = self.finally_defers.clone();
                for fin_body in defers.iter().rev() {
                    if self.current_block_terminated() { break; }
                    self.push_scope();
                    for s in fin_body { if self.compile_stmt(s)? { break; } }
                    self.pop_scope();
                }

                if !self.try_saved_tops.is_empty() && !self.current_block_terminated() {
                    let i32_ty = self.ctx.i32_type();
                    let depth_gv = self.get_or_create_eh_depth();
                    let outermost_depth_alloca = self.try_saved_tops[0];
                    if let Ok(saved) = self.builder.build_load(i32_ty, outermost_depth_alloca, "ret_eh_depth") {
                        let _ = self.builder.build_store(depth_gv.as_pointer_value(), saved);
                    }
                }

                self.arc_release_all_scopes_except(skip_var_name.as_deref())?;

                match ret_val {
                    None => {
                        let cur = self.cur_fn.unwrap();
                        match cur.get_type().get_return_type() {
                            None => {
                                self.builder.build_return(None)
                                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                            }
                            Some(rt) => {
                                use inkwell::types::BasicTypeEnum;
                                let zero_val: inkwell::values::BasicValueEnum = match rt {
                                    BasicTypeEnum::IntType(t)   => t.const_int(0, false).into(),
                                    BasicTypeEnum::FloatType(t) => t.const_float(0.0).into(),
                                    BasicTypeEnum::PointerType(t) => t.const_null().into(),
                                    _ => self.ctx.i64_type().const_int(0, false).into(),
                                };
                                self.builder.build_return(Some(&zero_val))
                                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                            }
                        }
                    }
                    Some(v) => {
                        // Cast return value to match function signature (e.g. i64→i32 for C main)
                        let final_val: inkwell::values::BasicValueEnum = if let Some(cur_fn) = self.cur_fn {
                            let fn_ret_ty = cur_fn.get_type().get_return_type();
                            match (v, fn_ret_ty) {
                                (inkwell::values::BasicValueEnum::IntValue(iv),
                                 Some(inkwell::types::BasicTypeEnum::IntType(t)))
                                    if iv.get_type().get_bit_width() != t.get_bit_width() => {
                                    if iv.get_type().get_bit_width() > t.get_bit_width() {
                                        self.builder.build_int_truncate(iv, t, "ret_cast")
                                            .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                                    } else {
                                        self.builder.build_int_s_extend(iv, t, "ret_cast")
                                            .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                                    }
                                }
                                _ => v,
                            }
                        } else { v };
                        self.builder.build_return(Some(&final_val))
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                    }
                }
                Ok(true)
            }

            Stmt::VarDecl { ty, name, value, volatile, .. } => {
                let is_volatile = *volatile;
                let (class_name, elem_class) = match ty {
                    Type::Named(n) if self.struct_types.contains_key(n.as_str()) => {
                        (Some(n.clone()), None)
                    }
                    Type::Nullable(inner) => match inner.as_ref() {
                        Type::Named(n) if self.struct_types.contains_key(n.as_str()) => {
                            (Some(n.clone()), None)
                        }
                        Type::List(li) => {
                            let ec = match li.as_ref() {
                                Type::Named(n)  => Some(n.clone()),
                                Type::Str       => Some("String".to_string()),
                                _ => None,
                            };
                            (Some("__List".to_string()), ec)
                        }
                        _ => (None, None),
                    },
                    Type::List(inner) => {
                        let ec = match inner.as_ref() {
                            Type::Named(n)   => Some(n.clone()),
                            Type::Str        => Some("String".to_string()),
                            Type::Integer    => Some("Integer".to_string()),
                            Type::Float      => Some("Float".to_string()),
                            Type::Boolean    => Some("Boolean".to_string()),
                            _ => None,
                        };
                        (Some("__List".to_string()), ec)
                    }
                    Type::HashMap(..) | Type::Map(..) | Type::TreeMap(..) => {
                        (Some("__HashMap".to_string()), None)
                    }
                    Type::Pair(..) => (Some("__Pair".to_string()), None),
                    _ => (None, None),
                };

                let llvm_ty = if class_name.is_some() {
                    Some(BasicTypeEnum::from(self.ctx.ptr_type(AddressSpace::default())))
                } else if let Type::Named(n) = ty {
                    if self.is_enum(n) {
                        Some(BasicTypeEnum::from(self.ctx.i32_type()))
                    } else {
                        self.llvm_type(ty)
                    }
                } else {
                    self.llvm_type(ty)
                };

                if let Some(llvm_ty) = llvm_ty {
                    // Hoist alloca to function entry block so loop-local vars
                    // don't grow the stack on every iteration (STATUS_STACK_OVERFLOW).
                    let alloca = {
                        let cur_block = self.builder.get_insert_block().unwrap();
                        let entry = self.cur_fn.unwrap().get_first_basic_block().unwrap();
                        if let Some(first_instr) = entry.get_first_instruction() {
                            self.builder.position_before(&first_instr);
                        } else {
                            self.builder.position_at_end(entry);
                        }
                        let a = self.builder.build_alloca(llvm_ty, name)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        // Zero-initialize to prevent ARC release on garbage stack data
                        let zero: BasicValueEnum = match llvm_ty {
                            BasicTypeEnum::IntType(t)     => t.const_int(0, false).into(),
                            BasicTypeEnum::FloatType(t)   => t.const_float(0.0).into(),
                            BasicTypeEnum::PointerType(t) => t.const_null().into(),
                            BasicTypeEnum::StructType(t)  => t.const_zero().into(),
                            BasicTypeEnum::ArrayType(t)   => t.const_zero().into(),
                            _ => self.ctx.i64_type().const_int(0, false).into(),
                        };
                        self.builder.build_store(a, zero)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        self.builder.position_at_end(cur_block);
                        a
                    };
                    if let Some(init_expr) = value {
                        if let Some(val) = self.compile_expr(init_expr)? {
                            let coerced = self.coerce_value(val, llvm_ty)?;
                            let store = self.builder.build_store(alloca, coerced)
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                            if is_volatile { let _ = store.set_volatile(true); }

                            if let Some(cn) = &class_name {
                                if !matches!(cn.as_str(), "__List" | "__HashMap" | "__Pair") {
                                    if let BasicValueEnum::PointerValue(ptr) = coerced {
                                        let cn_owned = cn.clone();
                                        self.arc_retain_ptr(ptr, &cn_owned)?;
                                    }
                                }
                            }
                        }
                    }
                    if elem_class.is_some() || matches!(class_name.as_deref(), Some("__List" | "__HashMap" | "__Pair")) {
                        self.define_collection_var(name, alloca, llvm_ty, class_name, elem_class);
                    } else if let Type::Named(n) = ty {
                        let n = n.clone();
                        if self.is_enum(&n) {
                            self.define_enum_var(name, alloca, llvm_ty, n);
                        } else {
                            self.define_var_with_class(name, alloca, llvm_ty, class_name);
                        }
                    } else {
                        self.define_var_with_class(name, alloca, llvm_ty, class_name);
                    }
                }
                Ok(false)
            }

            Stmt::ExprStmt(e) => {
                self.compile_expr(e)?;
                Ok(false)
            }

            Stmt::If { hint, cond, then, else_if, else_ } => {
                self.compile_if(hint.as_ref(), cond, then, else_if, else_)
            }

            Stmt::While { cond, body } => {
                self.compile_while(cond, body)
            }

            Stmt::For { init, cond, step, body } => {
                self.compile_for(init, cond, step, body)
            }

            Stmt::ForEach { ty, name, iter, body } => {
                self.compile_for_each(ty, name, iter, body)
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

            Stmt::TryCatch { try_body, catches, finally_body } => {
                self.declare_setjmp();
                self.declare_longjmp();
                self.declare_strcmp();

                let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                let i32_ty = self.ctx.i32_type();
                let cur_fn = self.cur_fn.unwrap();

                let depth_gv = self.get_or_create_eh_depth();
                let saved_depth = self.builder.build_load(i32_ty, depth_gv.as_pointer_value(), "saved_depth")
                    .map_err(|e| CodeGenError::new(e.to_string()))?
                    .into_int_value();

                let saved_depth_alloca = self.builder.build_alloca(i32_ty, "saved_depth_slot")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_store(saved_depth_alloca, saved_depth)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.try_saved_tops.push(saved_depth_alloca
                    .as_instruction_value().map(|_| saved_depth_alloca)
                    .unwrap_or(saved_depth_alloca));

                let new_depth = self.builder.build_int_add(saved_depth, i32_ty.const_int(1, false), "new_depth")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_store(depth_gv.as_pointer_value(), new_depth)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;

                let jmpbuf_ptr = self.get_jmpbuf_ptr(saved_depth)?;

                let sj_name = self.setjmp_fn_name();
                let setjmp_fn = self.module.get_function(sj_name).unwrap();
                #[cfg(target_os = "windows")]
                let setjmp_args: &[inkwell::values::BasicMetadataValueEnum] = &[
                    jmpbuf_ptr.into(),
                    self.ctx.ptr_type(inkwell::AddressSpace::default()).const_null().into(),
                ];
                #[cfg(not(target_os = "windows"))]
                let setjmp_args: &[inkwell::values::BasicMetadataValueEnum] = &[jmpbuf_ptr.into()];
                let setjmp_r = self.builder.build_call(setjmp_fn, setjmp_args, "setjmp_r")
                    .map_err(|e| CodeGenError::new(e.to_string()))?
                    .try_as_basic_value().basic().unwrap().into_int_value();

                let try_body_bb   = self.ctx.append_basic_block(cur_fn, "try.body");
                let catch_disp_bb = self.ctx.append_basic_block(cur_fn, "catch.dispatch");
                let finally_bb    = self.ctx.append_basic_block(cur_fn, "try.finally");
                let after_bb      = self.ctx.append_basic_block(cur_fn, "try.after");

                let is_ex = self.builder.build_int_compare(
                    inkwell::IntPredicate::NE, setjmp_r, i32_ty.const_int(0, false), "is_ex")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_conditional_branch(is_ex, catch_disp_bb, try_body_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;

                self.builder.position_at_end(try_body_bb);

                if let Some(fin) = finally_body {
                    self.finally_defers.push(fin.clone());
                }

                let mut try_returned = false;
                self.push_scope();
                for s in try_body {
                    if self.compile_stmt(s)? { try_returned = true; break; }
                }
                self.pop_scope();

                if finally_body.is_some() { self.finally_defers.pop(); }

                if !try_returned && !self.current_block_terminated() {
                    let sd = self.builder.build_load(i32_ty, saved_depth_alloca, "sd_restore")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_store(depth_gv.as_pointer_value(), sd)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_unconditional_branch(finally_bb)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                }

                self.builder.position_at_end(catch_disp_bb);

                let ex_type_gv = self.get_or_create_eh_global("__arimo_ex_type");
                let ex_msg_gv  = self.get_or_create_eh_global("__arimo_ex_msg");
                let ex_type_val = self.builder.build_load(ptr_ty, ex_type_gv.as_pointer_value(), "ex_type")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let ex_msg_val = self.builder.build_load(ptr_ty, ex_msg_gv.as_pointer_value(), "ex_msg")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;

                let rethrow_bb = self.ctx.append_basic_block(cur_fn, "catch.rethrow");
                let strcmp_fn  = self.module.get_function("strcmp").unwrap();

                for (i, catch) in catches.iter().enumerate() {
                    let catch_body_bb = self.ctx.append_basic_block(cur_fn, &format!("catch.{}.body", i));
                    let next_bb = if i + 1 < catches.len() {
                        self.ctx.append_basic_block(cur_fn, &format!("catch.{}.check", i + 1))
                    } else {
                        rethrow_bb
                    };

                    let catch_type = match &catch.exception_type {
                        crate::ast::Type::Named(n) => n.clone(),
                        _ => "Exception".to_string(),
                    };

                    if catch_type == "Exception" {
                        self.builder.build_unconditional_branch(catch_body_bb)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                    } else {
                        let catch_type_ptr = self.build_global_string(&catch_type)?;
                        let cmp = self.builder.build_call(
                            strcmp_fn, &[ex_type_val.into(), catch_type_ptr.into()], "type_cmp")
                            .map_err(|e| CodeGenError::new(e.to_string()))?
                            .try_as_basic_value().basic().unwrap().into_int_value();
                        let matched = self.builder.build_int_compare(
                            inkwell::IntPredicate::EQ, cmp, i32_ty.const_int(0, false), "type_match")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        self.builder.build_conditional_branch(matched, catch_body_bb, next_bb)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                    }

                    self.builder.position_at_end(catch_body_bb);
                    self.push_scope();

                    if !catch.name.is_empty() {
                        let alloca = self.builder.build_alloca(ptr_ty, &catch.name)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        if ex_msg_val.is_pointer_value() {
                            self.builder.build_store(alloca, ex_msg_val.into_pointer_value())
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                        }
                        self.define_var(&catch.name, alloca, ptr_ty.into());
                    }

                    let mut catch_ret = false;
                    for s in &catch.body {
                        if self.compile_stmt(s)? { catch_ret = true; break; }
                    }
                    self.pop_scope();

                    if !catch_ret && !self.current_block_terminated() {
                        let sd = self.builder.build_load(i32_ty, saved_depth_alloca, "sd_catch")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        self.builder.build_store(depth_gv.as_pointer_value(), sd)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        self.builder.build_unconditional_branch(finally_bb)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                    }

                    if i + 1 < catches.len() {
                        self.builder.position_at_end(next_bb);
                    }
                }

                self.builder.position_at_end(rethrow_bb);
                {
                    let sd = self.builder.build_load(i32_ty, saved_depth_alloca, "sd_rethrow")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                        .into_int_value();
                    self.builder.build_store(depth_gv.as_pointer_value(), sd)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;

                    let rethrow_null_bb    = self.ctx.append_basic_block(cur_fn, "rethrow.uncaught");
                    let rethrow_longjmp_bb = self.ctx.append_basic_block(cur_fn, "rethrow.longjmp");

                    let has_parent = self.builder.build_int_compare(
                        inkwell::IntPredicate::SGT, sd, i32_ty.const_int(0, false), "has_parent")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_conditional_branch(has_parent, rethrow_longjmp_bb, rethrow_null_bb)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;

                    self.builder.position_at_end(rethrow_null_bb);
                    self.declare_printf();
                    let fmt = self.build_global_string("Uncaught exception: %s\n")?;
                    let printf = self.module.get_function("printf").unwrap();
                    let ex_type2 = self.builder.build_load(ptr_ty, ex_type_gv.as_pointer_value(), "ex_t2")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_call(printf, &[fmt.into(), ex_type2.into()], "")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let abort_fn = self.module.get_function("abort").unwrap_or_else(|| {
                        let ft = self.ctx.void_type().fn_type(&[], false);
                        self.module.add_function("abort", ft, None)
                    });
                    self.builder.build_call(abort_fn, &[], "")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_unreachable()
                        .map_err(|e| CodeGenError::new(e.to_string()))?;

                    self.builder.position_at_end(rethrow_longjmp_bb);
                    let parent_slot = self.builder.build_int_sub(sd, i32_ty.const_int(1, false), "parent_slot")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let parent_jmpbuf = self.get_jmpbuf_ptr(parent_slot)?;
                    let longjmp_fn = self.module.get_function("longjmp").unwrap();
                    let one = i32_ty.const_int(1, false);
                    self.builder.build_call(longjmp_fn, &[parent_jmpbuf.into(), one.into()], "")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_unreachable()
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                }

                self.builder.position_at_end(finally_bb);
                if let Some(fin) = finally_body {
                    self.push_scope();
                    for s in fin { if self.compile_stmt(s)? { break; } }
                    self.pop_scope();
                }
                if !self.current_block_terminated() {
                    self.builder.build_unconditional_branch(after_bb)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                }

                self.builder.position_at_end(after_bb);
                self.try_saved_tops.pop();

                Ok(false)
            }

            Stmt::Throw(expr) => {
                self.declare_setjmp();
                self.declare_longjmp();
                self.declare_printf();

                let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                let i32_ty = self.ctx.i32_type();

                let (type_name_str, msg_opt) = match expr {
                    Expr::ConstructorCall { class, args, .. } => {
                        let name = class.clone();
                        let msg = if let Some(first) = args.first() {
                            self.compile_expr(first)?
                        } else {
                            None
                        };
                        (name, msg)
                    }
                    _ => ("Exception".to_string(), None),
                };

                let type_name_ptr = self.build_global_string(&type_name_str)?;
                let ex_type_gv = self.get_or_create_eh_global("__arimo_ex_type");
                self.builder.build_store(ex_type_gv.as_pointer_value(), type_name_ptr)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;

                let ex_msg_gv = self.get_or_create_eh_global("__arimo_ex_msg");
                if let Some(msg_val) = msg_opt {
                    if msg_val.is_pointer_value() {
                        self.builder.build_store(ex_msg_gv.as_pointer_value(), msg_val.into_pointer_value())
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                    }
                } else {
                    let empty = self.build_global_string("")?;
                    self.builder.build_store(ex_msg_gv.as_pointer_value(), empty)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                }

                let depth_gv = self.get_or_create_eh_depth();
                let depth_val = self.builder.build_load(i32_ty, depth_gv.as_pointer_value(), "throw_depth")
                    .map_err(|e| CodeGenError::new(e.to_string()))?
                    .into_int_value();

                let cur_fn = self.cur_fn.unwrap();
                let do_longjmp_bb = self.ctx.append_basic_block(cur_fn, "throw.longjmp");
                let do_abort_bb   = self.ctx.append_basic_block(cur_fn, "throw.uncaught");

                let has_frame = self.builder.build_int_compare(
                    inkwell::IntPredicate::SGT, depth_val, i32_ty.const_int(0, false), "has_frame")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_conditional_branch(has_frame, do_longjmp_bb, do_abort_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;

                self.builder.position_at_end(do_abort_bb);
                {
                    let fmt = self.build_global_string("Uncaught exception %s: %s\n")?;
                    let printf = self.module.get_function("printf").unwrap();
                    let msg_reload = self.builder.build_load(ptr_ty, ex_msg_gv.as_pointer_value(), "msg_r")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_call(printf, &[fmt.into(), type_name_ptr.into(), msg_reload.into()], "")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let abort_fn = self.module.get_function("abort").unwrap_or_else(|| {
                        let ft = self.ctx.void_type().fn_type(&[], false);
                        self.module.add_function("abort", ft, None)
                    });
                    self.builder.build_call(abort_fn, &[], "")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_unreachable()
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                }

                self.builder.position_at_end(do_longjmp_bb);
                {
                    let slot = self.builder.build_int_sub(depth_val, i32_ty.const_int(1, false), "throw_slot")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let jmpbuf_ptr = self.get_jmpbuf_ptr(slot)?;
                    let longjmp_fn = self.module.get_function("longjmp").unwrap();
                    let one = i32_ty.const_int(1, false);
                    self.builder.build_call(longjmp_fn, &[jmpbuf_ptr.into(), one.into()], "")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_unreachable()
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                }

                Ok(true)
            }

            Stmt::Switch { expr, cases } => {
                self.compile_switch(expr, cases)
            }

            Stmt::Asm(code) => {
                self.compile_asm(code)?;
                Ok(false)
            }

            Stmt::Defer(expr) => {
                if let Some(frame) = self.defer_stack.last_mut() {
                    frame.push(*expr.clone());
                }
                Ok(false)
            }

            Stmt::Break => {
                if let Some(&exit_bb) = self.loop_exit_bbs.last() {
                    self.arc_release_all_scopes()?;
                    self.builder.build_unconditional_branch(exit_bb)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }

            Stmt::Continue => {
                if let Some(&cont_bb) = self.loop_continue_bbs.last() {
                    self.arc_release_all_scopes()?;
                    self.builder.build_unconditional_branch(cont_bb)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }

    fn compile_if(
        &mut self,
        hint    : Option<&BranchHint>,
        cond    : &Expr,
        then    : &[Stmt],
        else_if : &[(Expr, Vec<Stmt>)],
        else_   : &Option<Vec<Stmt>>,
    ) -> CgResult<bool> {
        let cur_fn = self.cur_fn.unwrap();
        let cond_val = self.compile_expr(cond)?
            .ok_or_else(|| CodeGenError::new(format!(
                "if condition has no value — fn: {}, cond: {:?}",
                self.cur_fn.map(|f| f.get_name().to_str().unwrap_or("?").to_string()).unwrap_or_default(),
                cond
            )))?;
        let cond_bool = self.to_bool(cond_val)?;

        let then_bb  = self.ctx.append_basic_block(cur_fn, "if.then");
        let merge_bb = self.ctx.append_basic_block(cur_fn, "if.merge");
        let else_bb  = if else_.is_some() || !else_if.is_empty() {
            self.ctx.append_basic_block(cur_fn, "if.else")
        } else {
            merge_bb
        };

        let branch = self.builder.build_conditional_branch(cond_bool, then_bb, else_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        if let Some(h) = hint {
            let i32_ty = self.ctx.i32_type();
            let (then_w, else_w) = match h {
                BranchHint::Likely   => (2000u64, 1u64),
                BranchHint::Unlikely => (1u64, 2000u64),
            };
            let kind = self.ctx.get_kind_id("prof");
            let bw_str  = self.ctx.metadata_string("branch_weights");
            let tw_node = self.ctx.metadata_node(&[i32_ty.const_int(then_w, false).into()]);
            let ew_node = self.ctx.metadata_node(&[i32_ty.const_int(else_w, false).into()]);
            let prof_node = self.ctx.metadata_node(&[bw_str.into(), tw_node.into(), ew_node.into()]);
            branch.set_metadata(prof_node, kind)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }

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

        let mut else_returned = false;
        if else_bb != merge_bb {
            self.builder.position_at_end(else_bb);
            if !else_if.is_empty() {
                // compile else-if chain as nested if inside else block
                let (ei_cond, ei_body) = &else_if[0];
                let remaining = &else_if[1..];
                let ei_returned = self.compile_if(None, ei_cond, ei_body, remaining, else_)?;
                if ei_returned { else_returned = true; }
                // compile_if positions builder at its own merge_bb; we need to link to outer merge
                if !else_returned && !self.current_block_terminated() {
                    self.builder.build_unconditional_branch(merge_bb)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                }
            } else {
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
            }
        }

        self.builder.position_at_end(merge_bb);

        if then_returned && else_returned {
            self.builder.build_unreachable()
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            return Ok(true);
        }

        Ok(false)
    }

    fn compile_while(&mut self, cond: &Expr, body: &[Stmt]) -> CgResult<bool> {
        let cur_fn  = self.cur_fn.unwrap();
        let cond_bb = self.ctx.append_basic_block(cur_fn, "while.cond");
        let body_bb = self.ctx.append_basic_block(cur_fn, "while.body");
        let exit_bb = self.ctx.append_basic_block(cur_fn, "while.exit");

        self.loop_exit_bbs.push(exit_bb);
        self.loop_continue_bbs.push(cond_bb);

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(cond_bb);
        let cond_val = self.compile_expr(cond)?
            .ok_or_else(|| CodeGenError::new(format!(
                "while condition has no value — fn: {}, cond: {:?}",
                self.cur_fn.map(|f| f.get_name().to_str().unwrap_or("?").to_string()).unwrap_or_default(),
                cond
            )))?;
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

        self.loop_exit_bbs.pop();
        self.loop_continue_bbs.pop();
        self.builder.position_at_end(exit_bb);
        Ok(false)
    }

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

        self.loop_exit_bbs.push(exit_bb);
        self.loop_continue_bbs.push(step_bb);

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

        self.loop_exit_bbs.pop();
        self.loop_continue_bbs.pop();
        self.builder.position_at_end(exit_bb);
        self.pop_scope();
        Ok(false)
    }

    fn compile_switch(&mut self, expr: &Expr, cases: &[SwitchCase]) -> CgResult<bool> {
        let val = self.compile_expr(expr)?;
        let cur_fn = self.cur_fn.unwrap();
        let exit_bb = self.ctx.append_basic_block(cur_fn, "switch.exit");

        let mut all_cases_return = true;

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
                    all_cases_return = false;
                    self.builder.build_unconditional_branch(exit_bb)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                }
                self.builder.position_at_end(next_bb);
            }
        }

        self.builder.build_unconditional_branch(exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(exit_bb);

        if all_cases_return && !cases.is_empty() {
            self.builder.build_unreachable()
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            return Ok(true);
        }

        Ok(false)
    }

    fn compile_match(
        &mut self,
        expr : &Expr,
        arms : &[MatchArm],
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        let cur_fn   = self.cur_fn.unwrap();
        let match_val = match self.compile_expr(expr)? {
            Some(v) => v,
            None    => return Ok(None),
        };

        let i64_ty   = self.ctx.i64_type();
        let res_alloca = self.builder.build_alloca(i64_ty, "match_res")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(res_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let merge_bb = self.ctx.append_basic_block(cur_fn, "match.end");

        for arm in arms {
            match &arm.pattern {
                MatchPattern::Wildcard | MatchPattern::Binding(_) => {
                    if let Some(guard) = &arm.guard {
                        let body_bb = self.ctx.append_basic_block(cur_fn, "match.arm");
                        let skip_bb = self.ctx.append_basic_block(cur_fn, "match.skip");
                        self.push_scope();
                        if let MatchPattern::Binding(name) = &arm.pattern {
                            let alloca = self.builder.build_alloca(i64_ty, name)
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                            let mval64 = self.value_to_i64(match_val)?;
                            self.builder.build_store(alloca, mval64)
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                            self.define_var(name, alloca, i64_ty.into());
                        }
                        let gv = self.compile_expr(guard)?.unwrap_or(i64_ty.const_int(0, false).into());
                        let gb = self.to_bool(gv)?;
                        self.builder.build_conditional_branch(gb, body_bb, skip_bb)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;

                        self.builder.position_at_end(body_bb);
                        let body_val = self.compile_expr(&arm.body)?;
                        if let Some(v) = body_val {
                            let stored = self.value_to_i64(v)?;
                            self.builder.build_store(res_alloca, stored)
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                        }
                        if !self.current_block_terminated() {
                            self.builder.build_unconditional_branch(merge_bb)
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                        }
                        self.pop_scope();
                        self.builder.position_at_end(skip_bb);
                    } else {
                        self.push_scope();
                        if let MatchPattern::Binding(name) = &arm.pattern {
                            let alloca = self.builder.build_alloca(i64_ty, name)
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                            let mval64 = self.value_to_i64(match_val)?;
                            self.builder.build_store(alloca, mval64)
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                            self.define_var(name, alloca, i64_ty.into());
                        }
                        let body_val = self.compile_expr(&arm.body)?;
                        self.pop_scope();
                        if let Some(v) = body_val {
                            let stored = self.value_to_i64(v)?;
                            self.builder.build_store(res_alloca, stored)
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                        }
                        if !self.current_block_terminated() {
                            self.builder.build_unconditional_branch(merge_bb)
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                        }
                        break;
                    }
                }

                MatchPattern::StrLit(_) | MatchPattern::Multi(_) => {
                    let patterns: Vec<String> = match &arm.pattern {
                        MatchPattern::StrLit(s) => vec![s.clone()],
                        MatchPattern::Multi(pats) => pats.iter().filter_map(|p| {
                            if let MatchPattern::StrLit(s) = p { Some(s.clone()) } else { None }
                        }).collect(),
                        _ => vec![],
                    };
                    let int_pats: Vec<i64> = match &arm.pattern {
                        MatchPattern::IntLit(n) => vec![*n],
                        MatchPattern::Multi(pats) => pats.iter().filter_map(|p| {
                            if let MatchPattern::IntLit(n) = p { Some(*n) } else { None }
                        }).collect(),
                        _ => vec![],
                    };

                    let then_bb = self.ctx.append_basic_block(cur_fn, "match.arm");
                    let next_bb = self.ctx.append_basic_block(cur_fn, "match.next");

                    let mut or_cond: Option<inkwell::values::IntValue<'ctx>> = None;
                    let str_ptr = match match_val { BasicValueEnum::PointerValue(p) => Some(p), _ => None };
                    if !patterns.is_empty() {
                        self.declare_string_fns();
                        let strcmp = self.module.get_function("strcmp").unwrap();
                        for pat_s in &patterns {
                            let pat_ptr = self.build_global_string(pat_s)?;
                            if let Some(sp) = str_ptr {
                                let r = self.builder.build_call(strcmp, &[sp.into(), pat_ptr.into()], "sc")
                                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                                if let Some(BasicValueEnum::IntValue(cmp)) = r.try_as_basic_value().basic() {
                                    let eq = self.builder.build_int_compare(
                                        inkwell::IntPredicate::EQ, cmp, self.ctx.i32_type().const_int(0, false), "seq"
                                    ).map_err(|e| CodeGenError::new(e.to_string()))?;
                                    or_cond = Some(match or_cond {
                                        None => eq,
                                        Some(prev) => self.builder.build_or(prev, eq, "or").map_err(|e| CodeGenError::new(e.to_string()))?,
                                    });
                                }
                            }
                        }
                    }
                    for n in &int_pats {
                        if let BasicValueEnum::IntValue(mv) = match_val {
                            let nv = i64_ty.const_int(*n as u64, *n < 0);
                            let eq = self.builder.build_int_compare(inkwell::IntPredicate::EQ, mv, nv, "ieq")
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                            or_cond = Some(match or_cond {
                                None => eq,
                                Some(prev) => self.builder.build_or(prev, eq, "or").map_err(|e| CodeGenError::new(e.to_string()))?,
                            });
                        }
                    }

                    let cond = or_cond.unwrap_or(self.ctx.bool_type().const_int(0, false));
                    self.builder.build_conditional_branch(cond, then_bb, next_bb)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;

                    self.builder.position_at_end(then_bb);
                    self.push_scope();
                    let body_val = self.compile_expr(&arm.body)?;
                    self.pop_scope();
                    if let Some(v) = body_val {
                        let stored = self.value_to_i64(v)?;
                        self.builder.build_store(res_alloca, stored)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                    }
                    if !self.current_block_terminated() {
                        self.builder.build_unconditional_branch(merge_bb)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                    }
                    self.builder.position_at_end(next_bb);
                }

                MatchPattern::IntLit(n) => {
                    let then_bb = self.ctx.append_basic_block(cur_fn, "match.arm");
                    let next_bb = self.ctx.append_basic_block(cur_fn, "match.next");
                    let cond = if let BasicValueEnum::IntValue(mv) = match_val {
                        let nv = i64_ty.const_int(*n as u64, *n < 0);
                        self.builder.build_int_compare(inkwell::IntPredicate::EQ, mv, nv, "ieq")
                            .map_err(|e| CodeGenError::new(e.to_string()))?
                    } else {
                        self.ctx.bool_type().const_int(0, false)
                    };
                    self.builder.build_conditional_branch(cond, then_bb, next_bb)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.position_at_end(then_bb);
                    self.push_scope();
                    let body_val = self.compile_expr(&arm.body)?;
                    self.pop_scope();
                    if let Some(v) = body_val {
                        let stored = self.value_to_i64(v)?;
                        self.builder.build_store(res_alloca, stored)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                    }
                    if !self.current_block_terminated() {
                        self.builder.build_unconditional_branch(merge_bb)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                    }
                    self.builder.position_at_end(next_bb);
                }

                MatchPattern::Variant { enum_name, variant, bindings } => {
                    let variant_val = self.enum_variant_value(enum_name, variant)
                        .map(|v| self.ctx.i32_type().const_int(v as u64, false));

                    let then_bb = self.ctx.append_basic_block(cur_fn, "match.arm");
                    let next_bb = self.ctx.append_basic_block(cur_fn, "match.next");

                    let cond = if let Some(vv) = variant_val {
                        let match_i32 = match match_val {
                            BasicValueEnum::IntValue(iv) => {
                                if iv.get_type().get_bit_width() > 32 {
                                    self.builder.build_int_truncate(iv, self.ctx.i32_type(), "mi32")
                                        .map_err(|e| CodeGenError::new(e.to_string()))?
                                } else { iv }
                            }
                            _ => self.ctx.i32_type().const_int(0, false),
                        };
                        self.builder.build_int_compare(
                            inkwell::IntPredicate::EQ, match_i32, vv, "arm_eq"
                        ).map_err(|e| CodeGenError::new(e.to_string()))?
                    } else {
                        self.ctx.bool_type().const_int(0, false)
                    };

                    self.builder.build_conditional_branch(cond, then_bb, next_bb)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;

                    self.builder.position_at_end(then_bb);
                    self.push_scope();

                    let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                    for b in bindings {
                        let alloca = self.builder.build_alloca(i64_ty, b)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        self.builder.build_store(alloca, i64_ty.const_int(0, false))
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        self.define_var(b, alloca, i64_ty.into());
                    }
                    let _ = ptr_ty;

                    let body_val = self.compile_expr(&arm.body)?;
                    self.pop_scope();

                    if let Some(v) = body_val {
                        let stored = self.value_to_i64(v)?;
                        self.builder.build_store(res_alloca, stored)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                    }
                    if !self.current_block_terminated() {
                        self.builder.build_unconditional_branch(merge_bb)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                    }

                    self.builder.position_at_end(next_bb);
                }
            }
        }

        if !self.current_block_terminated() {
            self.builder.build_unconditional_branch(merge_bb)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }

        self.builder.position_at_end(merge_bb);
        let res = self.builder.build_load(i64_ty, res_alloca, "match_val")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        Ok(Some(res))
    }

    fn compile_expr(&mut self, expr: &Expr) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        match expr {
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
                self.compile_str_interp(parts)
            }
            Expr::NullLit => {
                let v = self.ctx.ptr_type(AddressSpace::default()).const_null();
                Ok(Some(v.into()))
            }

            Expr::Ident(name) => {
                if let Some(slot) = self.lookup_var(name).cloned() {
                    let v = self.builder.build_load(slot.ty, slot.ptr, name)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    Ok(Some(v))
                } else {
                    Ok(None)
                }
            }

            Expr::StaticCall { class, method, args }
                if class == "IO" && (method == "print" || method == "println") =>
            {
                self.compile_io_print(args)?;
                if method == "println" {
                    let newline = self.build_global_string("\n")?;
                    let printf = self.module.get_function("printf").unwrap();
                    self.builder.build_call(printf, &[newline.into()], "")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    // flush stdout so output is visible even if crash follows
                    let fflush_fn = if let Some(f) = self.module.get_function("fflush") { f } else {
                        let i32_ty = self.ctx.i32_type();
                        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                        self.module.add_function("fflush", i32_ty.fn_type(&[ptr_ty.into()], false), None)
                    };
                    let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                    self.builder.build_call(fflush_fn, &[ptr_ty.const_null().into()], "")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                }
                Ok(None)
            }

            Expr::StaticCall { class, method, args } if
                matches!(class.as_str(), "IO"|"Math"|"Time"|"Memory"|"Env") =>
            {
                self.compile_stdlib_call(class, method, args)
            }

            Expr::StaticCall { class, method, args } => {
                self.compile_static_call(class, method, args)
            }

            Expr::MethodCall { object, method, args } => {
                let obj_class = self.infer_object_class(object);
                if let Some(cls) = obj_class.as_deref() {
                    if matches!(cls, "__List" | "__HashMap" | "__Pair") {
                        let cls_owned = cls.to_string();
                        return self.compile_collection_method(object, &cls_owned, method, args);
                    }
                }
                if let Expr::Ident(class_name) = object.as_ref() {
                    let fn_name = format!("{}_{}", class_name, method);
                    if self.fns.contains_key(&fn_name)
                        || self.module.get_function(&fn_name).is_some()
                    {
                        return self.compile_static_call(class_name, method, args);
                    }
                }
                if Self::is_string_method(method) {
                    let str_val = self.compile_expr(object)?;
                    if let Some(v @ BasicValueEnum::PointerValue(_)) = str_val {
                        if obj_class.is_none() || obj_class.as_deref() == Some("String") {
                            let args_cloned = args.to_vec();
                            return self.compile_string_method(v, method, &args_cloned);
                        }
                    }
                }
                // Extension method dispatch for primitive types
                {
                    let obj_val = self.compile_expr(object)?;
                    if let Some(ov) = obj_val {
                        let type_prefix = match ov {
                            BasicValueEnum::IntValue(iv) if iv.get_type().get_bit_width() == 1 => Some("Boolean"),
                            BasicValueEnum::IntValue(_)  => Some("Integer"),
                            BasicValueEnum::FloatValue(_) => Some("Float"),
                            _ => None,
                        };
                        if let Some(prefix) = type_prefix {
                            let fn_name = format!("{}_{}", prefix, method);
                            if let Some(ext_fn) = self.fns.get(&fn_name).copied()
                                .or_else(|| self.module.get_function(&fn_name))
                            {
                                let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum> = vec![ov.into()];
                                for a in args {
                                    if let Some(v) = self.compile_expr(a)? { call_args.push(v.into()); }
                                }
                                let call = self.builder.build_call(ext_fn, &call_args, "ext_call")
                                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                                return Ok(call.try_as_basic_value().basic());
                            }
                        }
                    }
                }
                self.compile_instance_method_call(object, method, args)
            }

            Expr::ConstructorCall { class, args } => {
                match class.as_str() {
                    "List" | "List.of" | "List.empty" => {
                        let f = self.module.get_function("arc_list_new").unwrap();
                        let call = self.builder.build_call(f, &[], "list")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        return Ok(call.try_as_basic_value().basic());
                    }
                    "HashMap" | "TreeMap" | "HashMap.of" | "HashMap.create"
                    | "TreeMap.create" => {
                        let f = self.module.get_function("arc_map_new").unwrap();
                        let call = self.builder.build_call(f, &[], "map")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        return Ok(call.try_as_basic_value().basic());
                    }
                    "Pair" | "Pair.of" => {
                        let compiled: Vec<BasicValueEnum<'ctx>> = args.iter()
                            .filter_map(|a| self.compile_expr(a).ok().flatten())
                            .collect();
                        let fst_raw = compiled.get(0).copied();
                        let snd_raw = compiled.get(1).copied();
                        let fst = if let Some(v) = fst_raw {
                            self.value_to_i64(v)?
                        } else {
                            self.ctx.i64_type().const_int(0, false)
                        };
                        let snd = if let Some(v) = snd_raw {
                            self.value_to_i64(v)?
                        } else {
                            self.ctx.i64_type().const_int(0, false)
                        };
                        let f = self.module.get_function("arc_pair_new").unwrap();
                        let call = self.builder.build_call(f, &[fst.into(), snd.into()], "pair")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        return Ok(call.try_as_basic_value().basic());
                    }
                    _ => {}
                }

                let fn_ptr_info = self.lookup_var(class.as_str())
                    .map(|s| (s.ptr, s.ty));
                if let Some((fn_ptr_ptr, fn_ptr_ty)) = fn_ptr_info {
                    let fn_ptr_val = self.builder.build_load(fn_ptr_ty, fn_ptr_ptr, "fnptr")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let is_fat  = matches!(fn_ptr_val, BasicValueEnum::StructValue(_));
                    let is_bare = matches!(fn_ptr_val, BasicValueEnum::PointerValue(_));
                    if is_fat || is_bare {
                        let i64_ty = self.ctx.i64_type();
                        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                        let mut compiled_args: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::new();
                        for a in args {
                            if let Some(v) = self.compile_expr(a)? {
                                let promoted: inkwell::values::BasicMetadataValueEnum = match v {
                                    BasicValueEnum::IntValue(iv) if iv.get_type().get_bit_width() < 64 => {
                                        self.builder.build_int_z_extend(iv, i64_ty, "arg_ze")
                                            .map(|v| v.into())
                                            .unwrap_or_else(|_| iv.into())
                                    }
                                    other => other.into(),
                                };
                                compiled_args.push(promoted);
                            }
                        }
                        let (fn_p, cl_ptr) = self.extract_fn_closure(fn_ptr_val)?;
                        let param_types: Vec<inkwell::types::BasicMetadataTypeEnum> =
                            compiled_args.iter().map(|_| i64_ty.into()).collect();
                        if is_fat {
                            compiled_args.push(cl_ptr.into());
                            let mut fp_types = param_types;
                            fp_types.push(ptr_ty.into());
                            let fn_type = i64_ty.fn_type(&fp_types, false);
                            let call = self.builder.build_indirect_call(fn_type, fn_p, &compiled_args, "lambda_call")
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                            return Ok(call.try_as_basic_value().basic());
                        } else {
                            let fn_type = i64_ty.fn_type(&param_types, false);
                            let call = self.builder.build_indirect_call(fn_type, fn_p, &compiled_args, "lambda_call")
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                            return Ok(call.try_as_basic_value().basic());
                        }
                    }
                }

                let compiled: Vec<BasicValueEnum<'ctx>> = args.iter()
                    .filter_map(|a| self.compile_expr(a).ok().flatten())
                    .collect();
                let meta: Vec<inkwell::values::BasicMetadataValueEnum> =
                    compiled.iter().map(|v| (*v).into()).collect();

                // Try as a direct extern function call (e.g. fopen, remove, ftell...)
                if let Some(extern_fn) = self.module.get_function(class.as_str()) {
                    let call = self.builder.build_call(extern_fn, &meta, "extern_call")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    return Ok(call.try_as_basic_value().basic());
                }

                let ctor_name = format!("{}_new", class);
                if let Some(ctor_fn) = self.fns.get(&ctor_name).copied()
                    .or_else(|| self.module.get_function(&ctor_name))
                {
                    let call = self.builder.build_call(ctor_fn, &meta, "obj")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    Ok(call.try_as_basic_value().basic())
                } else {
                    Ok(None)
                }
            }

            Expr::BinOp { op, left, right } => {
                self.compile_binop(op, left, right)
            }

            Expr::UnaryOp { op, expr } => {
                self.compile_unary(op, expr)
            }

            Expr::Cast { expr, ty } => {
                let val = self.compile_expr(expr)?;
                if let (Some(v), Some(target_ty)) = (val, self.llvm_type(ty)) {
                    let casted = self.build_cast(v, target_ty)?;
                    Ok(Some(casted))
                } else {
                    Ok(None)
                }
            }

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

            Expr::Index { object, index } => {
                let obj = self.compile_expr(object)?;
                let idx = self.compile_expr(index)?;
                if let (Some(o), Some(i)) = (obj, idx) {
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

            Expr::This => {
                if let Some(slot) = self.lookup_var("this").cloned() {
                    let v = self.builder.build_load(slot.ty, slot.ptr, "this_val")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    Ok(Some(v))
                } else {
                    Ok(None)
                }
            }
            Expr::Super => {
                if let Some(slot) = self.lookup_var("this").cloned() {
                    let v = self.builder.build_load(slot.ty, slot.ptr, "super_val")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    Ok(Some(v))
                } else {
                    Ok(None)
                }
            }

            Expr::Await(inner) => self.compile_expr(inner),

            Expr::NullCoalesce { left, right } => {
                let cur_fn = self.cur_fn.unwrap();
                let lv = match self.compile_expr(left)? {
                    Some(v) => v,
                    None    => return Ok(None),
                };
                let lbool = self.to_bool(lv)?;
                let then_bb  = self.ctx.append_basic_block(cur_fn, "coal.left");
                let else_bb  = self.ctx.append_basic_block(cur_fn, "coal.right");
                let merge_bb = self.ctx.append_basic_block(cur_fn, "coal.merge");
                self.builder.build_conditional_branch(lbool, then_bb, else_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;

                self.builder.position_at_end(then_bb);
                self.builder.build_unconditional_branch(merge_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;

                self.builder.position_at_end(else_bb);
                let rv = self.compile_expr(right)?;
                self.builder.build_unconditional_branch(merge_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let else_end = self.builder.get_insert_block().unwrap();

                self.builder.position_at_end(merge_bb);
                if let Some(r) = rv {
                    let phi = self.builder.build_phi(lv.get_type(), "coal_phi")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    phi.add_incoming(&[(&lv, then_bb), (&r, else_end)]);
                    Ok(Some(phi.as_basic_value()))
                } else {
                    Ok(Some(lv))
                }
            }

            Expr::Match { expr, arms } => {
                self.compile_match(expr, arms)
            }

            Expr::Lambda { params, body } => {
                self.compile_general_lambda(params, body)
            }

            Expr::FieldAccess { object, field } => {
                if let Expr::Ident(class_or_enum) = object.as_ref() {
                    if let Some(val) = self.enum_variant_value(class_or_enum, field) {
                        let iv = self.ctx.i32_type().const_int(val as u64, false);
                        return Ok(Some(iv.into()));
                    }
                    let global_name = format!("{}_{}", class_or_enum, field);
                    if let Some(gv) = self.module.get_global(&global_name) {
                        let ptr  = gv.as_pointer_value();
                        if let Some(g_ty) = self.any_to_basic(gv.get_value_type()) {
                            let val = self.builder.build_load(g_ty, ptr, field)
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                            return Ok(Some(val));
                        }
                    }
                }
                self.compile_field_load(object, field)
            }
            Expr::NullSafeAccess { object, field, args } => {
                let cur_fn = match self.cur_fn { Some(f) => f, None => return Ok(None) };
                let obj_val = match self.compile_expr(object)? {
                    Some(v) => v,
                    None    => return Ok(None),
                };
                let i64_ty = self.ctx.i64_type();
                let ptr_ty = self.ctx.ptr_type(AddressSpace::default());

                let ok_bb    = self.ctx.append_basic_block(cur_fn, "ns.ok");
                let merge_bb = self.ctx.append_basic_block(cur_fn, "ns.merge");

                let null_src_bb = self.builder.get_insert_block().unwrap();
                let is_null = match obj_val {
                    BasicValueEnum::PointerValue(p) => {
                        let pi = self.builder.build_ptr_to_int(p, i64_ty, "ns_pi")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        self.builder.build_int_compare(
                            inkwell::IntPredicate::EQ, pi, i64_ty.const_int(0, false), "ns_null"
                        ).map_err(|e| CodeGenError::new(e.to_string()))?
                    }
                    _ => self.ctx.bool_type().const_int(0, false),
                };
                self.builder.build_conditional_branch(is_null, merge_bb, ok_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;

                self.builder.position_at_end(ok_bb);
                let result = match args {
                    Some(call_args) => {
                        let class_name = self.infer_object_class(object);
                        if let Some(cn) = &class_name {
                            let fn_name = format!("{}_{}", cn, field);
                            if let Some(fn_val) = self.fns.get(&fn_name).copied()
                                .or_else(|| self.module.get_function(&fn_name))
                            {
                                let mut cargs: Vec<inkwell::values::BasicMetadataValueEnum> = vec![obj_val.into()];
                                for a in call_args {
                                    if let Some(v) = self.compile_expr(a)? { cargs.push(v.into()); }
                                }
                                let call = self.builder.build_call(fn_val, &cargs, "ns_call")
                                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                                call.try_as_basic_value().basic()
                            } else { None }
                        } else { None }
                    }
                    None => self.compile_field_load(object, field)?,
                };
                let ok_val = result.unwrap_or_else(|| ptr_ty.const_null().into());
                let ok_src_bb = self.builder.get_insert_block().unwrap();
                self.builder.build_unconditional_branch(merge_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;

                self.builder.position_at_end(merge_bb);

                let merged: BasicValueEnum<'ctx> = match ok_val {
                    BasicValueEnum::PointerValue(_) => {
                        let phi = self.builder.build_phi(ptr_ty, "ns_phi")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        phi.add_incoming(&[
                            (&ok_val,                     ok_src_bb),
                            (&BasicValueEnum::from(ptr_ty.const_null()), null_src_bb),
                        ]);
                        phi.as_basic_value()
                    }
                    BasicValueEnum::IntValue(iv) => {
                        let it = iv.get_type();
                        let phi = self.builder.build_phi(it, "ns_phi")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        phi.add_incoming(&[
                            (&ok_val,                          ok_src_bb),
                            (&BasicValueEnum::from(it.const_int(0, false)), null_src_bb),
                        ]);
                        phi.as_basic_value()
                    }
                    other => other,
                };
                Ok(Some(merged))
            }
        }
    }

    fn compile_str_interp(
        &mut self,
        parts: &[StringPart],
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        if self.module.get_function("sprintf").is_none() {
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let i32_ty = self.ctx.i32_type();
            let ft = i32_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], true);
            self.module.add_function("sprintf", ft, None);
        }

        let mut fmt_str   = String::new();
        let mut interp_vals: Vec<BasicValueEnum<'ctx>> = Vec::new();

        for part in parts {
            match part {
                StringPart::Text(t) => {
                    fmt_str.push_str(&t.replace('%', "%%"));
                }
                StringPart::Interp(inner_expr) => {
                    if let Some(ep) = self.try_enum_label(inner_expr) {
                        fmt_str.push_str("%s");
                        interp_vals.push(ep.into());
                    } else if let Some(val) = self.compile_expr(inner_expr)? {
                        // Boolean i1 → "true"/"false" BEFORE spec is determined
                        if let BasicValueEnum::IntValue(iv) = val {
                            if iv.get_type().get_bit_width() == 1 {
                                let ts = self.build_global_string("true")?;
                                let fs = self.build_global_string("false")?;
                                let sel = self.builder.build_select(iv, ts, fs, "bool_str")
                                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                                fmt_str.push_str("%s");
                                interp_vals.push(sel.into());
                                continue;
                            }
                        }
                        let spec = match val {
                            BasicValueEnum::IntValue(iv) => {
                                match iv.get_type().get_bit_width() {
                                    64 => "%lld",
                                    _  => "%d",
                                }
                            }
                            BasicValueEnum::FloatValue(_) => "%g",
                            BasicValueEnum::PointerValue(_) => "%s",
                            _ => "%d",
                        };
                        fmt_str.push_str(spec);
                        let promoted = match val {
                            BasicValueEnum::FloatValue(f) => {
                                let f64ty = self.ctx.f64_type();
                                if f.get_type().get_bit_width() < 64 {
                                    self.builder.build_float_ext(f, f64ty, "fpext")
                                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                                } else { val }
                            }
                            BasicValueEnum::IntValue(iv)
                                if iv.get_type().get_bit_width() < 32 =>
                            {
                                let i32ty = self.ctx.i32_type();
                                self.builder.build_int_z_extend(iv, i32ty, "zext")
                                    .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                            }
                            _ => val,
                        };
                        interp_vals.push(promoted);
                    } else {
                        fmt_str.push_str("(null)");
                    }
                }
            }
        }

        let malloc_fn = self.module.get_function("malloc").unwrap();
        let i64_ty    = self.ctx.i64_type();
        let sz        = i64_ty.const_int(1024, false);
        let buf_call  = self.builder.build_call(malloc_fn, &[sz.into()], "interp_buf")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let buf_ptr = match buf_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(None),
        };

        let fmt_ptr = self.build_global_string(&fmt_str)?;
        let sprintf_fn = self.module.get_function("sprintf").unwrap();
        let mut sprintf_args: Vec<inkwell::values::BasicMetadataValueEnum> =
            vec![buf_ptr.into(), fmt_ptr.into()];
        for v in interp_vals { sprintf_args.push(v.into()); }
        self.builder.build_call(sprintf_fn, &sprintf_args, "")
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        Ok(Some(buf_ptr.into()))
    }

    fn compile_stdlib_call(
        &mut self,
        class  : &str,
        method : &str,
        args   : &[Expr],
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        match (class, method) {
            ("IO", "error") => {
                // print to stderr
                let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                let i32_ty = self.ctx.i32_type();
                let fprintf_fn = if let Some(f) = self.module.get_function("fprintf") { f } else {
                    self.module.add_function("fprintf",
                        i32_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], true), None)
                };
                #[cfg(target_os = "windows")]
                let stderr_ptr = {
                    let iob_fn = if let Some(f) = self.module.get_function("__acrt_iob_func") { f } else {
                        self.module.add_function("__acrt_iob_func",
                            ptr_ty.fn_type(&[i32_ty.into()], false), None)
                    };
                    let call = self.builder.build_call(iob_fn,
                        &[i32_ty.const_int(2, false).into()], "stderr_v")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    match call.try_as_basic_value().basic() {
                        Some(BasicValueEnum::PointerValue(p)) => p,
                        _ => ptr_ty.const_null(),
                    }
                };
                #[cfg(not(target_os = "windows"))]
                let stderr_ptr = {
                    let stderr_global = if let Some(g) = self.module.get_global("stderr") {
                        g
                    } else {
                        self.module.add_global(ptr_ty, None, "stderr")
                    };
                    self.builder.build_load(ptr_ty, stderr_global.as_pointer_value(), "stderr_ptr")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                        .into_pointer_value()
                };
                for a in args {
                    if let Some(v) = self.compile_expr(a)? {
                        if let BasicValueEnum::PointerValue(sp) = v {
                            let fmt_s = self.build_global_string("%s\n")?;
                            self.builder.build_call(fprintf_fn, &[stderr_ptr.into(), fmt_s.into(), sp.into()], "")
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                        }
                    }
                }
                let fflush_fn = if let Some(f) = self.module.get_function("fflush") { f } else {
                    self.module.add_function("fflush",
                        i32_ty.fn_type(&[ptr_ty.into()], false), None)
                };
                self.builder.build_call(fflush_fn, &[ptr_ty.const_null().into()], "")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(None)
            }
            ("IO", "print") => {
                self.compile_io_print(args)?;
                Ok(None)
            }
            ("IO", "read") => {
                let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                let i64_ty = self.ctx.i64_type();
                let i32_ty = self.ctx.i32_type();
                let i8_ty  = self.ctx.i8_type();

                let scanf_fn = if let Some(f) = self.module.get_function("scanf") { f } else {
                    self.module.add_function("scanf",
                        i32_ty.fn_type(&[ptr_ty.into()], true), None)
                };
                let strlen_fn = if let Some(f) = self.module.get_function("strlen") { f } else {
                    self.module.add_function("strlen",
                        i64_ty.fn_type(&[ptr_ty.into()], false), None)
                };
                let malloc_fn = self.module.get_function("malloc")
                    .ok_or_else(|| CodeGenError::new("malloc not declared"))?;
                let memcpy_fn = if let Some(f) = self.module.get_function("memcpy") { f } else {
                    self.module.add_function("memcpy",
                        ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), i64_ty.into()], false), None)
                };

                let buf_alloca = self.builder.build_alloca(i8_ty.array_type(4096), "io_rbuf")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let buf_ptr = self.builder.build_pointer_cast(buf_alloca, ptr_ty, "buf")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;

                // zero-init buffer so empty input returns ""
                let memset_fn = if let Some(f) = self.module.get_function("memset") { f } else {
                    self.module.add_function("memset",
                        ptr_ty.fn_type(&[ptr_ty.into(), i32_ty.into(), i64_ty.into()], false), None)
                };
                self.builder.build_call(memset_fn, &[
                    buf_ptr.into(),
                    i32_ty.const_int(0, false).into(),
                    i64_ty.const_int(4096, false).into(),
                ], "").map_err(|e| CodeGenError::new(e.to_string()))?;

                // scanf("%4095[^\n]", buf) — reads up to newline, strips it
                let fmt = self.build_global_string("%4095[^\n]")?;
                let fmt_ptr = self.builder.build_pointer_cast(fmt, ptr_ty, "fmt_ptr")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_call(scanf_fn, &[fmt_ptr.into(), buf_ptr.into()], "sc")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                // consume the newline
                let nl_fmt = self.build_global_string("%*c")?;
                let nl_fmt_ptr = self.builder.build_pointer_cast(nl_fmt, ptr_ty, "nlfmt")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_call(scanf_fn, &[nl_fmt_ptr.into()], "sc_nl")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;

                // malloc + memcpy → heap string
                let slen_call = self.builder.build_call(strlen_fn, &[buf_ptr.into()], "slen")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let slen = match slen_call.try_as_basic_value().basic() {
                    Some(BasicValueEnum::IntValue(iv)) =>
                        self.builder.build_int_z_extend_or_bit_cast(iv, i64_ty, "slen64")
                            .map_err(|e| CodeGenError::new(e.to_string()))?,
                    _ => i64_ty.const_int(0, false),
                };
                let asz = self.builder.build_int_add(slen, i64_ty.const_int(1, false), "asz")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let heap_call = self.builder.build_call(malloc_fn, &[asz.into()], "rdh")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let heap_ptr = match heap_call.try_as_basic_value().basic() {
                    Some(BasicValueEnum::PointerValue(p)) => p,
                    _ => ptr_ty.const_null(),
                };
                self.builder.build_call(memcpy_fn, &[heap_ptr.into(), buf_ptr.into(), asz.into()], "")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(Some(heap_ptr.into()))
            }

            ("Math", "sqrt") => {
                self.declare_math_fns();
                if let Some(arg) = args.first() {
                    if let Some(v) = self.compile_expr(arg)? {
                        let f64v = self.to_f64(v)?;
                        let sqrt_fn = self.module.get_function("sqrt").unwrap();
                        let r = self.builder.build_call(sqrt_fn, &[f64v.into()], "sqrt")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        return Ok(r.try_as_basic_value().basic());
                    }
                }
                Ok(None)
            }
            ("Math", "abs") => {
                if let Some(arg) = args.first() {
                    if let Some(v) = self.compile_expr(arg)? {
                        match v {
                            BasicValueEnum::FloatValue(f) => {
                                self.declare_math_fns();
                                let fabs = self.module.get_function("fabs").unwrap();
                                let r = self.builder.build_call(fabs, &[f.into()], "abs")
                                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                                return Ok(r.try_as_basic_value().basic());
                            }
                            BasicValueEnum::IntValue(i) => {
                                let neg = self.builder.build_int_neg(i, "neg")
                                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                                let cmp = self.builder.build_int_compare(
                                    inkwell::IntPredicate::SGE, i, neg, "cmp"
                                ).map_err(|e| CodeGenError::new(e.to_string()))?;
                                let r = self.builder.build_select(cmp, i, neg, "abs")
                                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                                return Ok(Some(r));
                            }
                            _ => {}
                        }
                    }
                }
                Ok(None)
            }
            ("Math", "pow") => {
                self.declare_math_fns();
                let vals: Vec<_> = args.iter()
                    .filter_map(|a| self.compile_expr(a).ok().flatten())
                    .collect();
                if vals.len() >= 2 {
                    let a = self.to_f64(vals[0])?;
                    let b = self.to_f64(vals[1])?;
                    let pow_fn = self.module.get_function("pow").unwrap();
                    let r = self.builder.build_call(pow_fn, &[a.into(), b.into()], "pow")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    return Ok(r.try_as_basic_value().basic());
                }
                Ok(None)
            }
            ("Math", "PI") | ("Math", "E") => {
                let val = if method == "PI" { std::f64::consts::PI } else { std::f64::consts::E };
                Ok(Some(self.ctx.f64_type().const_float(val).into()))
            }

            ("Time", "now") => {
                let s = self.build_global_string("2026-01-01")?;
                Ok(Some(s.into()))
            }
            ("Time", "generateId") => {
                self.declare_arc_runtime();
                let id_fn = self.module.get_function("arc_generate_id");
                if let Some(f) = id_fn {
                    let r = self.builder.build_call(f, &[], "id")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    Ok(r.try_as_basic_value().basic())
                } else {
                    let s = self.build_global_string("id-1")?;
                    Ok(Some(s.into()))
                }
            }

            ("Memory", "alloc") => {
                if let Some(arg) = args.first() {
                    if let Some(v) = self.compile_expr(arg)? {
                        let malloc = self.module.get_function("malloc")
                            .ok_or_else(|| CodeGenError::new("malloc not declared"))?;
                        let size = self.to_i64(v)?;
                        let r = self.builder.build_call(malloc, &[size.into()], "alloc")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        return Ok(r.try_as_basic_value().basic());
                    }
                }
                Ok(None)
            }
            ("Memory", "free") => {
                if let Some(arg) = args.first() {
                    if let Some(v) = self.compile_expr(arg)? {
                        let free = self.module.get_function("free")
                            .or_else(|| {
                                let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                                let ft = self.ctx.void_type().fn_type(&[ptr_ty.into()], false);
                                Some(self.module.add_function("free", ft, None))
                            }).unwrap();
                        self.builder.build_call(free, &[v.into()], "")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                    }
                }
                Ok(None)
            }
            ("Memory", "set") | ("Memory", "copy") => {
                for a in args { self.compile_expr(a)?; }
                Ok(None)
            }

            ("Env", "exit") => {
                let i32_ty = self.ctx.i32_type();
                let code_i32 = if let Some(arg) = args.first() {
                    if let Some(v) = self.compile_expr(arg)? {
                        match v {
                            BasicValueEnum::IntValue(i) =>
                                self.builder.build_int_truncate_or_bit_cast(i, i32_ty, "exit_code")
                                    .map_err(|e| CodeGenError::new(e.to_string()))?,
                            _ => i32_ty.const_int(0, false),
                        }
                    } else { i32_ty.const_int(0, false) }
                } else { i32_ty.const_int(0, false) };
                let exit_fn = self.module.get_function("exit").unwrap_or_else(|| {
                    let ft = self.ctx.void_type().fn_type(&[i32_ty.into()], false);
                    self.module.add_function("exit", ft, None)
                });
                self.builder.build_call(exit_fn, &[code_i32.into()], "")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(None)
            }

            ("Env", "platform") => {
                let s = self.build_global_string("windows")?;
                Ok(Some(s.into()))
            }

            ("Env", "exePath") => {
                let i64_ty = self.ctx.i64_type();
                let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                let argv_gv = self.get_or_create_env_global("__arimo_argv", ptr_ty.into());
                let argv = self.builder.build_load(ptr_ty, argv_gv.as_pointer_value(), "argv")
                    .map_err(|e| CodeGenError::new(e.to_string()))?.into_pointer_value();
                let arg0_ptr_ptr = unsafe {
                    self.builder.build_gep(ptr_ty, argv, &[i64_ty.const_int(0, false)], "arg0pp")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                };
                let arg0 = self.builder.build_load(ptr_ty, arg0_ptr_ptr, "arg0")
                    .map_err(|e| CodeGenError::new(e.to_string()))?.into_pointer_value();
                Ok(Some(arg0.into()))
            }

            ("Env", "args") => {
                let i32_ty = self.ctx.i32_type();
                let i64_ty = self.ctx.i64_type();
                let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                let argc_gv = self.get_or_create_env_global("__arimo_argc", i32_ty.into());
                let argv_gv = self.get_or_create_env_global("__arimo_argv", ptr_ty.into());
                let argc = self.builder.build_load(i32_ty, argc_gv.as_pointer_value(), "argc")
                    .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
                let argv = self.builder.build_load(ptr_ty, argv_gv.as_pointer_value(), "argv")
                    .map_err(|e| CodeGenError::new(e.to_string()))?.into_pointer_value();

                let list_new_fn = self.module.get_function("arc_list_new").unwrap();
                let list_append_fn = self.module.get_function("arc_list_append").unwrap();
                let list_call = self.builder.build_call(list_new_fn, &[], "env_args_list")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let list_ptr = match list_call.try_as_basic_value().basic() {
                    Some(BasicValueEnum::PointerValue(p)) => p,
                    _ => return Ok(None),
                };

                let cur_fn = match self.cur_fn { Some(f) => f, None => return Ok(None) };
                let idx_alloca = self.builder.build_alloca(i32_ty, "args_i")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_store(idx_alloca, i32_ty.const_int(0, false))
                    .map_err(|e| CodeGenError::new(e.to_string()))?;

                let cond_bb = self.ctx.append_basic_block(cur_fn, "args.cond");
                let body_bb = self.ctx.append_basic_block(cur_fn, "args.body");
                let exit_bb = self.ctx.append_basic_block(cur_fn, "args.exit");

                self.builder.build_unconditional_branch(cond_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.position_at_end(cond_bb);
                let idx = self.builder.build_load(i32_ty, idx_alloca, "ai")
                    .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
                let cond = self.builder.build_int_compare(inkwell::IntPredicate::SLT, idx, argc, "ac")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_conditional_branch(cond, body_bb, exit_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;

                self.builder.position_at_end(body_bb);
                let idx64 = self.builder.build_int_s_extend(idx, i64_ty, "ai64")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let arg_ptr_ptr = unsafe {
                    self.builder.build_gep(ptr_ty, argv, &[idx64], "argpp")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                };
                let arg_ptr = self.builder.build_load(ptr_ty, arg_ptr_ptr, "argp")
                    .map_err(|e| CodeGenError::new(e.to_string()))?.into_pointer_value();
                let arg_i64 = self.builder.build_ptr_to_int(arg_ptr, i64_ty, "argpi")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_call(list_append_fn, &[list_ptr.into(), arg_i64.into()], "")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let next = self.builder.build_int_add(idx, i32_ty.const_int(1, false), "ai_inc")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_store(idx_alloca, next)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_unconditional_branch(cond_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;

                self.builder.position_at_end(exit_bb);
                Ok(Some(list_ptr.into()))
            }

            _ => {
                for a in args { self.compile_expr(a)?; }
                Ok(None)
            }
        }
    }

    fn declare_collection_runtime(&mut self) {
        self.declare_malloc();
        self.declare_strcmp();

        self.pre_declare_arc_runtime_fns();

        let prev = self.builder.get_insert_block();

        self.gen_arc_list_new();
        self.gen_arc_list_append();
        self.gen_arc_list_length();
        self.gen_arc_list_get();
        self.gen_arc_list_set();
        self.gen_arc_list_filter();
        self.gen_arc_map_new();
        self.gen_arc_map_set();
        self.gen_arc_map_get_or_default();
        self.gen_arc_pair_new();
        self.gen_arc_pair_first();
        self.gen_arc_pair_second();

        if let Some(bb) = prev { self.builder.position_at_end(bb); }
    }

    fn declare_string_fns(&mut self) {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64_ty = self.ctx.i64_type();
        let i32_ty = self.ctx.i32_type();
        let i8_ty  = self.ctx.i8_type();

        if self.module.get_function("strlen").is_none() {
            let ft = i64_ty.fn_type(&[ptr_ty.into()], false);
            self.module.add_function("strlen", ft, None);
        }
        if self.module.get_function("strstr").is_none() {
            let ft = ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
            self.module.add_function("strstr", ft, None);
        }
        if self.module.get_function("strncmp").is_none() {
            let ft = i32_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), i64_ty.into()], false);
            self.module.add_function("strncmp", ft, None);
        }
        if self.module.get_function("strcat").is_none() {
            let ft = ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
            self.module.add_function("strcat", ft, None);
        }
        if self.module.get_function("strcpy").is_none() {
            let ft = ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
            self.module.add_function("strcpy", ft, None);
        }
        if self.module.get_function("toupper").is_none() {
            let ft = i32_ty.fn_type(&[i32_ty.into()], false);
            self.module.add_function("toupper", ft, None);
        }
        if self.module.get_function("tolower").is_none() {
            let ft = i32_ty.fn_type(&[i32_ty.into()], false);
            self.module.add_function("tolower", ft, None);
        }
        if self.module.get_function("strtok").is_none() {
            let ft = ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
            self.module.add_function("strtok", ft, None);
        }
        if self.module.get_function("strtol").is_none() {
            let ft = i64_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), i32_ty.into()], false);
            self.module.add_function("strtol", ft, None);
        }
        if self.module.get_function("strtod").is_none() {
            let f64_ty = self.ctx.f64_type();
            let ft = f64_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
            self.module.add_function("strtod", ft, None);
        }
        let _ = i8_ty;
    }

    fn is_string_method(method: &str) -> bool {
        matches!(method, "length" | "contains" | "startsWith" | "endsWith" |
                         "compareTo" | "toUpper" | "toLower" | "trim" |
                         "split" | "indexOf" | "substring" | "replace" |
                         "parseInt" | "parseFloat" | "isEmpty" | "isBlank" |
                         "repeat" | "padStart" | "padEnd" | "chars" | "concat" |
                         "charCodeAt" | "charAt" | "toString")
    }

    fn compile_string_method(
        &mut self,
        str_val : BasicValueEnum<'ctx>,
        method  : &str,
        args    : &[Expr],
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        self.declare_string_fns();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64_ty = self.ctx.i64_type();
        let i32_ty = self.ctx.i32_type();
        let str_ptr = match str_val {
            BasicValueEnum::PointerValue(p) => p,
            _ => return Ok(None),
        };

        match method {
            "length" => {
                let strlen = self.module.get_function("strlen").unwrap();
                let r = self.builder.build_call(strlen, &[str_ptr.into()], "slen")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(r.try_as_basic_value().basic())
            }

            "contains" => {
                let arg = args.first().and_then(|a| self.compile_expr(a).ok().flatten())
                    .unwrap_or(ptr_ty.const_null().into());
                let arg_ptr = match arg {
                    BasicValueEnum::PointerValue(p) => p,
                    _ => ptr_ty.const_null(),
                };
                let strstr = self.module.get_function("strstr").unwrap();
                let r = self.builder.build_call(strstr, &[str_ptr.into(), arg_ptr.into()], "found")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                if let Some(BasicValueEnum::PointerValue(rp)) = r.try_as_basic_value().basic() {
                    let null = ptr_ty.const_null();
                    let neq = self.builder.build_int_compare(
                        inkwell::IntPredicate::NE,
                        self.builder.build_ptr_to_int(rp, i64_ty, "p2i")
                            .map_err(|e| CodeGenError::new(e.to_string()))?,
                        self.builder.build_ptr_to_int(null, i64_ty, "np2i")
                            .map_err(|e| CodeGenError::new(e.to_string()))?,
                        "contains"
                    ).map_err(|e| CodeGenError::new(e.to_string()))?;
                    return Ok(Some(neq.into()));
                }
                Ok(Some(self.ctx.bool_type().const_int(0, false).into()))
            }

            "startsWith" => {
                let arg = args.first().and_then(|a| self.compile_expr(a).ok().flatten())
                    .unwrap_or(ptr_ty.const_null().into());
                let arg_ptr = match arg {
                    BasicValueEnum::PointerValue(p) => p,
                    _ => ptr_ty.const_null(),
                };
                let strlen = self.module.get_function("strlen").unwrap();
                let strncmp = self.module.get_function("strncmp").unwrap();
                let prefix_len = self.builder.build_call(strlen, &[arg_ptr.into()], "pfxlen")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let plen = prefix_len.try_as_basic_value().basic()
                    .ok_or_else(|| CodeGenError::new("strlen returned void"))?;
                let r = self.builder.build_call(strncmp, &[str_ptr.into(), arg_ptr.into(), plen.into()], "sw_cmp")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                if let Some(BasicValueEnum::IntValue(cmp)) = r.try_as_basic_value().basic() {
                    let zero = i32_ty.const_int(0, false);
                    let eq = self.builder.build_int_compare(inkwell::IntPredicate::EQ, cmp, zero, "sw")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    return Ok(Some(eq.into()));
                }
                Ok(Some(self.ctx.bool_type().const_int(0, false).into()))
            }

            "endsWith" => {
                let arg = args.first().and_then(|a| self.compile_expr(a).ok().flatten())
                    .unwrap_or(ptr_ty.const_null().into());
                let arg_ptr = match arg {
                    BasicValueEnum::PointerValue(p) => p,
                    _ => ptr_ty.const_null(),
                };
                let strlen  = self.module.get_function("strlen").unwrap();
                let strncmp = self.module.get_function("strncmp").unwrap();
                let str_len = {
                    let r = self.builder.build_call(strlen, &[str_ptr.into()], "slen")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    match r.try_as_basic_value().basic() {
                        Some(BasicValueEnum::IntValue(v)) => v,
                        _ => return Ok(Some(self.ctx.bool_type().const_int(0, false).into())),
                    }
                };
                let suf_len = {
                    let r = self.builder.build_call(strlen, &[arg_ptr.into()], "suflen")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    match r.try_as_basic_value().basic() {
                        Some(BasicValueEnum::IntValue(v)) => v,
                        _ => return Ok(Some(self.ctx.bool_type().const_int(0, false).into())),
                    }
                };
                let offset = self.builder.build_int_sub(str_len, suf_len, "ew_off")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let i8_ty  = self.ctx.i8_type();
                let start_ptr = unsafe {
                    self.builder.build_gep(i8_ty, str_ptr, &[offset], "ew_ptr")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                };
                let r = self.builder.build_call(strncmp, &[start_ptr.into(), arg_ptr.into(), suf_len.into()], "ew_cmp")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                if let Some(BasicValueEnum::IntValue(cmp)) = r.try_as_basic_value().basic() {
                    let zero = i32_ty.const_int(0, false);
                    let eq = self.builder.build_int_compare(inkwell::IntPredicate::EQ, cmp, zero, "ew")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    return Ok(Some(eq.into()));
                }
                Ok(Some(self.ctx.bool_type().const_int(0, false).into()))
            }

            "compareTo" => {
                self.declare_strcmp();
                let arg = args.first().and_then(|a| self.compile_expr(a).ok().flatten())
                    .unwrap_or(ptr_ty.const_null().into());
                let arg_ptr = match arg {
                    BasicValueEnum::PointerValue(p) => p,
                    _ => ptr_ty.const_null(),
                };
                let strcmp = self.module.get_function("strcmp").unwrap();
                let r = self.builder.build_call(strcmp, &[str_ptr.into(), arg_ptr.into()], "cmp")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                if let Some(BasicValueEnum::IntValue(v)) = r.try_as_basic_value().basic() {
                    let extended = self.builder.build_int_s_extend(v, i64_ty, "cmp64")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    return Ok(Some(extended.into()));
                }
                Ok(None)
            }

            "toUpper" => {
                let result = self.build_str_case_convert(str_ptr, true)?;
                Ok(Some(result.into()))
            }

            "toLower" => {
                let result = self.build_str_case_convert(str_ptr, false)?;
                Ok(Some(result.into()))
            }

            "charCodeAt" => {
                let idx = args.first().and_then(|a| self.compile_expr(a).ok().flatten())
                    .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) } else { None })
                    .unwrap_or(i64_ty.const_int(0, false));
                let i8_ty = self.ctx.i8_type();
                let ch_ptr = unsafe {
                    self.builder.build_gep(i8_ty, str_ptr, &[idx], "ch_ptr")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                };
                let ch = self.builder.build_load(i8_ty, ch_ptr, "ch")
                    .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
                let ch64 = self.builder.build_int_z_extend(ch, i64_ty, "ch64")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(Some(ch64.into()))
            }

            "charAt" => {
                let idx = args.first().and_then(|a| self.compile_expr(a).ok().flatten())
                    .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) } else { None })
                    .unwrap_or(i64_ty.const_int(0, false));
                let i8_ty  = self.ctx.i8_type();
                let malloc = self.module.get_function("malloc").unwrap();
                let buf_call = self.builder.build_call(malloc, &[i64_ty.const_int(2, false).into()], "ch_buf")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let buf_ptr = match buf_call.try_as_basic_value().basic() {
                    Some(BasicValueEnum::PointerValue(p)) => p,
                    _ => return Ok(None),
                };
                let ch_ptr = unsafe {
                    self.builder.build_gep(i8_ty, str_ptr, &[idx], "ch_src")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                };
                let ch = self.builder.build_load(i8_ty, ch_ptr, "ch_v")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_store(buf_ptr, ch)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let null_gep = unsafe {
                    self.builder.build_gep(i8_ty, buf_ptr, &[i64_ty.const_int(1, false)], "ch_null")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                };
                self.builder.build_store(null_gep, i8_ty.const_int(0, false))
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(Some(buf_ptr.into()))
            }

            "toString" => {
                Ok(Some(str_ptr.into()))
            }

            "indexOf" => {
                let arg = args.first().and_then(|a| self.compile_expr(a).ok().flatten())
                    .unwrap_or(ptr_ty.const_null().into());
                let arg_ptr = match arg {
                    BasicValueEnum::PointerValue(p) => p,
                    _ => ptr_ty.const_null(),
                };
                let strstr = self.module.get_function("strstr").unwrap();
                let r = self.builder.build_call(strstr, &[str_ptr.into(), arg_ptr.into()], "idx_ptr")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                if let Some(BasicValueEnum::PointerValue(rp)) = r.try_as_basic_value().basic() {
                    let rp_int = self.builder.build_ptr_to_int(rp, i64_ty, "rp_i")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let base_int = self.builder.build_ptr_to_int(str_ptr, i64_ty, "base_i")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let null_val = i64_ty.const_int(0, false);
                    let is_null = self.builder.build_int_compare(
                        inkwell::IntPredicate::EQ, rp_int, null_val, "is_null"
                    ).map_err(|e| CodeGenError::new(e.to_string()))?;
                    let offset = self.builder.build_int_sub(rp_int, base_int, "offset")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let minus_one = i64_ty.const_int(u64::MAX, true);
                    let result = self.builder.build_select(is_null, minus_one, offset, "idx")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    return Ok(Some(result));
                }
                Ok(Some(i64_ty.const_int(u64::MAX, true).into()))
            }

            "split" => {
                let arg = args.first().and_then(|a| self.compile_expr(a).ok().flatten())
                    .unwrap_or(ptr_ty.const_null().into());
                let arg_ptr = match arg {
                    BasicValueEnum::PointerValue(p) => p,
                    _ => ptr_ty.const_null(),
                };
                let result = self.build_str_split(str_ptr, arg_ptr)?;
                Ok(Some(result.into()))
            }

            "parseInt" => {
                let strtol  = self.module.get_function("strtol").unwrap();
                let null_ptr = ptr_ty.const_null();
                let base10   = i32_ty.const_int(10, false);
                let r = self.builder.build_call(strtol, &[str_ptr.into(), null_ptr.into(), base10.into()], "parsed_i")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(r.try_as_basic_value().basic())
            }

            "parseFloat" => {
                let strtod = self.module.get_function("strtod").unwrap();
                let null_ptr = ptr_ty.const_null();
                let r = self.builder.build_call(strtod, &[str_ptr.into(), null_ptr.into()], "parsed_f")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(r.try_as_basic_value().basic())
            }

            "substring" => {
                let start = args.first().and_then(|a| self.compile_expr(a).ok().flatten())
                    .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) } else { None })
                    .unwrap_or(i64_ty.const_int(0, false));
                let end_val = args.get(1).and_then(|a| self.compile_expr(a).ok().flatten())
                    .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) } else { None })
                    .unwrap_or(i64_ty.const_int(0, false));
                let sub_len = self.builder.build_int_sub(end_val, start, "sub_len")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let sub_len1 = self.builder.build_int_add(sub_len, i64_ty.const_int(1, false), "sub_len1")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let malloc = self.module.get_function("malloc").unwrap();
                let buf_call = self.builder.build_call(malloc, &[sub_len1.into()], "sub_buf")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let buf_ptr = match buf_call.try_as_basic_value().basic() {
                    Some(BasicValueEnum::PointerValue(p)) => p,
                    _ => return Ok(None),
                };
                let i8_ty = self.ctx.i8_type();
                let src_start = unsafe {
                    self.builder.build_gep(i8_ty, str_ptr, &[start], "sub_src")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                };
                let memcpy_fn = self.module.get_function("memcpy").unwrap_or_else(|| {
                    let ptr = ptr_ty;
                    let ft  = ptr.fn_type(&[ptr.into(), ptr.into(), i64_ty.into()], false);
                    self.module.add_function("memcpy", ft, None)
                });
                self.builder.build_call(memcpy_fn, &[buf_ptr.into(), src_start.into(), sub_len.into()], "")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let null_gep = unsafe {
                    self.builder.build_gep(i8_ty, buf_ptr, &[sub_len], "sub_null_gep")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                };
                self.builder.build_store(null_gep, i8_ty.const_int(0, false))
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(Some(buf_ptr.into()))
            }

            "replace" => {
                let old_str = args.first().and_then(|a| self.compile_expr(a).ok().flatten())
                    .and_then(|v| if let BasicValueEnum::PointerValue(p) = v { Some(p) } else { None })
                    .unwrap_or(ptr_ty.const_null());
                let new_str = args.get(1).and_then(|a| self.compile_expr(a).ok().flatten())
                    .and_then(|v| if let BasicValueEnum::PointerValue(p) = v { Some(p) } else { None })
                    .unwrap_or(ptr_ty.const_null());
                let strlen = self.module.get_function("strlen").unwrap();
                let strstr = self.module.get_function("strstr").unwrap();
                let malloc = self.module.get_function("malloc").unwrap();
                let strcat = self.module.get_function("strcat").unwrap();
                let cur_fn = match self.cur_fn { Some(f) => f, None => return Ok(None) };

                let found_call = self.builder.build_call(strstr, &[str_ptr.into(), old_str.into()], "rep_search")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let found_ptr = match found_call.try_as_basic_value().basic() {
                    Some(BasicValueEnum::PointerValue(p)) => p,
                    _ => return Ok(Some(str_ptr.into())),
                };
                let rep_found_bb   = self.ctx.append_basic_block(cur_fn, "rep.found");
                let rep_nofound_bb = self.ctx.append_basic_block(cur_fn, "rep.nofound");
                let rep_end_bb     = self.ctx.append_basic_block(cur_fn, "rep.end");
                let is_null = self.builder.build_is_null(found_ptr, "rep_isnull")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_conditional_branch(is_null, rep_nofound_bb, rep_found_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;

                self.builder.position_at_end(rep_nofound_bb);
                self.builder.build_unconditional_branch(rep_end_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;

                self.builder.position_at_end(rep_found_bb);
                let found_i  = self.builder.build_ptr_to_int(found_ptr, i64_ty, "rep_fi")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let src_i    = self.builder.build_ptr_to_int(str_ptr, i64_ty, "rep_si")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let pfx_len  = self.builder.build_int_sub(found_i, src_i, "rep_pfx")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let old_len  = {
                    let r = self.builder.build_call(strlen, &[old_str.into()], "rep_ol").map_err(|e| CodeGenError::new(e.to_string()))?;
                    match r.try_as_basic_value().basic() { Some(BasicValueEnum::IntValue(v)) => v, _ => i64_ty.const_int(0, false) }
                };
                let new_len  = {
                    let r = self.builder.build_call(strlen, &[new_str.into()], "rep_nl").map_err(|e| CodeGenError::new(e.to_string()))?;
                    match r.try_as_basic_value().basic() { Some(BasicValueEnum::IntValue(v)) => v, _ => i64_ty.const_int(0, false) }
                };
                let src_len  = {
                    let r = self.builder.build_call(strlen, &[str_ptr.into()], "rep_sl").map_err(|e| CodeGenError::new(e.to_string()))?;
                    match r.try_as_basic_value().basic() { Some(BasicValueEnum::IntValue(v)) => v, _ => i64_ty.const_int(0, false) }
                };
                let sfx_len = self.builder.build_int_sub(src_len,
                    self.builder.build_int_add(pfx_len, old_len, "").map_err(|e| CodeGenError::new(e.to_string()))?,
                    "rep_sfxl").map_err(|e| CodeGenError::new(e.to_string()))?;
                let total = self.builder.build_int_add(
                    self.builder.build_int_add(pfx_len, new_len, "").map_err(|e| CodeGenError::new(e.to_string()))?,
                    self.builder.build_int_add(sfx_len, i64_ty.const_int(1, false), "").map_err(|e| CodeGenError::new(e.to_string()))?,
                    "rep_total"
                ).map_err(|e| CodeGenError::new(e.to_string()))?;
                let buf_call = self.builder.build_call(malloc, &[total.into()], "rep_buf")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let buf_ptr = match buf_call.try_as_basic_value().basic() {
                    Some(BasicValueEnum::PointerValue(p)) => p,
                    _ => { self.builder.build_unconditional_branch(rep_end_bb).map_err(|e| CodeGenError::new(e.to_string()))?; return Ok(Some(str_ptr.into())); },
                };
                let i8_ty = self.ctx.i8_type();
                let memcpy_fn = self.module.get_function("memcpy").unwrap_or_else(|| {
                    let ft = ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), i64_ty.into()], false);
                    self.module.add_function("memcpy", ft, None)
                });
                self.builder.build_call(memcpy_fn, &[buf_ptr.into(), str_ptr.into(), pfx_len.into()], "")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let buf_after_pfx = unsafe {
                    self.builder.build_gep(i8_ty, buf_ptr, &[pfx_len], "rep_apfx")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                };
                self.builder.build_store(buf_after_pfx, i8_ty.const_int(0, false))
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_call(strcat, &[buf_ptr.into(), new_str.into()], "")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let old_end = unsafe {
                    self.builder.build_gep(i8_ty, found_ptr, &[old_len], "rep_oe")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                };
                self.builder.build_call(strcat, &[buf_ptr.into(), old_end.into()], "")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_unconditional_branch(rep_end_bb)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;

                self.builder.position_at_end(rep_end_bb);
                let phi = self.builder.build_phi(ptr_ty, "rep_result")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                phi.add_incoming(&[(&str_ptr, rep_nofound_bb), (&buf_ptr, rep_found_bb)]);
                Ok(Some(phi.as_basic_value()))
            }

            "concat" => {
                let other = args.first().and_then(|a| self.compile_expr(a).ok().flatten())
                    .and_then(|v| if let BasicValueEnum::PointerValue(p) = v { Some(p) } else { None })
                    .unwrap_or(ptr_ty.const_null());
                let strlen = self.module.get_function("strlen").unwrap();
                let malloc  = self.module.get_function("malloc").unwrap();
                let llen = { let r = self.builder.build_call(strlen, &[str_ptr.into()], "llen").map_err(|e| CodeGenError::new(e.to_string()))?; match r.try_as_basic_value().basic() { Some(BasicValueEnum::IntValue(v)) => v, _ => i64_ty.const_int(0, false) } };
                let rlen = { let r = self.builder.build_call(strlen, &[other.into()], "rlen").map_err(|e| CodeGenError::new(e.to_string()))?; match r.try_as_basic_value().basic() { Some(BasicValueEnum::IntValue(v)) => v, _ => i64_ty.const_int(0, false) } };
                let total = self.builder.build_int_add(llen, rlen, "cat_total").map_err(|e| CodeGenError::new(e.to_string()))?;
                let total1 = self.builder.build_int_add(total, i64_ty.const_int(1, false), "cat_total1").map_err(|e| CodeGenError::new(e.to_string()))?;
                let buf_call = self.builder.build_call(malloc, &[total1.into()], "cat_buf").map_err(|e| CodeGenError::new(e.to_string()))?;
                let buf_ptr = match buf_call.try_as_basic_value().basic() {
                    Some(BasicValueEnum::PointerValue(p)) => p,
                    _ => return Ok(Some(str_ptr.into())),
                };
                let memcpy_fn = self.module.get_function("memcpy").unwrap_or_else(|| {
                    let ft = ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), i64_ty.into()], false);
                    self.module.add_function("memcpy", ft, None)
                });
                self.builder.build_call(memcpy_fn, &[buf_ptr.into(), str_ptr.into(), llen.into()], "").map_err(|e| CodeGenError::new(e.to_string()))?;
                let i8_ty = self.ctx.i8_type();
                let mid = unsafe { self.builder.build_gep(i8_ty, buf_ptr, &[llen], "cat_mid").map_err(|e| CodeGenError::new(e.to_string()))? };
                self.builder.build_call(memcpy_fn, &[mid.into(), other.into(), rlen.into()], "").map_err(|e| CodeGenError::new(e.to_string()))?;
                let end = unsafe { self.builder.build_gep(i8_ty, buf_ptr, &[total], "cat_end").map_err(|e| CodeGenError::new(e.to_string()))? };
                self.builder.build_store(end, i8_ty.const_int(0, false)).map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(Some(buf_ptr.into()))
            }

            "trim" => {
                let result = self.build_str_trim(str_ptr)?;
                Ok(Some(result.into()))
            }

            _ => {
                for a in args { self.compile_expr(a)?; }
                Ok(None)
            }
        }
    }

    fn build_str_trim(
        &mut self,
        src_ptr : inkwell::values::PointerValue<'ctx>,
    ) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        let i8_ty  = self.ctx.i8_type();
        let i64_ty = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let strlen = self.module.get_function("strlen").unwrap();
        let malloc = self.module.get_function("malloc").unwrap();
        let memcpy_fn = self.module.get_function("memcpy").unwrap_or_else(|| {
            let ft = ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), i64_ty.into()], false);
            self.module.add_function("memcpy", ft, None)
        });
        let cur_fn = match self.cur_fn { Some(f) => f, None => return Ok(src_ptr) };

        let len_call = self.builder.build_call(strlen, &[src_ptr.into()], "trim_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let len = match len_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => return Ok(src_ptr),
        };

        let start_alloca = self.builder.build_alloca(i64_ty, "trim_s")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let end_alloca   = self.builder.build_alloca(i64_ty, "trim_e")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let init_end = self.builder.build_int_sub(len, i64_ty.const_int(1, false), "init_e")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(start_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(end_alloca, init_end)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        macro_rules! is_ws {
            ($c:expr) => {{
                let sp = self.builder.build_int_compare(inkwell::IntPredicate::EQ, $c, i8_ty.const_int(32, false), "ws_sp").map_err(|e| CodeGenError::new(e.to_string()))?;
                let tb = self.builder.build_int_compare(inkwell::IntPredicate::EQ, $c, i8_ty.const_int(9, false), "ws_tb").map_err(|e| CodeGenError::new(e.to_string()))?;
                let lf = self.builder.build_int_compare(inkwell::IntPredicate::EQ, $c, i8_ty.const_int(10, false), "ws_lf").map_err(|e| CodeGenError::new(e.to_string()))?;
                let cr = self.builder.build_int_compare(inkwell::IntPredicate::EQ, $c, i8_ty.const_int(13, false), "ws_cr").map_err(|e| CodeGenError::new(e.to_string()))?;
                let w1 = self.builder.build_or(sp, tb, "w1").map_err(|e| CodeGenError::new(e.to_string()))?;
                let w2 = self.builder.build_or(lf, cr, "w2").map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_or(w1, w2, "ws").map_err(|e| CodeGenError::new(e.to_string()))?
            }};
        }

        // Forward scan: skip leading whitespace
        let fwd_cond = self.ctx.append_basic_block(cur_fn, "trim.fwd_cond");
        let fwd_body = self.ctx.append_basic_block(cur_fn, "trim.fwd_body");
        let rev_cond = self.ctx.append_basic_block(cur_fn, "trim.rev_cond");
        let rev_body = self.ctx.append_basic_block(cur_fn, "trim.rev_body");
        let trim_out = self.ctx.append_basic_block(cur_fn, "trim.out");

        self.builder.build_unconditional_branch(fwd_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(fwd_cond);
        let si = self.builder.build_load(i64_ty, start_alloca, "si")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let fwd_in = self.builder.build_int_compare(inkwell::IntPredicate::SLT, si, len, "fwd_in")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let fwd_char_ptr = unsafe { self.builder.build_gep(i8_ty, src_ptr, &[si], "fwd_cp")
            .map_err(|e| CodeGenError::new(e.to_string()))? };
        let fwd_c = self.builder.build_load(i8_ty, fwd_char_ptr, "fwd_c")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let fwd_ws = is_ws!(fwd_c);
        let fwd_cont = self.builder.build_and(fwd_in, fwd_ws, "fwd_cont")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(fwd_cont, fwd_body, rev_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(fwd_body);
        let si2 = self.builder.build_int_add(si, i64_ty.const_int(1, false), "si_inc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(start_alloca, si2)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(fwd_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // Reverse scan: skip trailing whitespace
        self.builder.position_at_end(rev_cond);
        let si_final = self.builder.build_load(i64_ty, start_alloca, "si_f")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let ei_cur = self.builder.build_load(i64_ty, end_alloca, "ei_c")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let rev_pos = self.builder.build_int_compare(inkwell::IntPredicate::SGE, ei_cur, si_final, "rev_pos")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let rev_char_ptr = unsafe { self.builder.build_gep(i8_ty, src_ptr, &[ei_cur], "rev_cp")
            .map_err(|e| CodeGenError::new(e.to_string()))? };
        let rev_c = self.builder.build_load(i8_ty, rev_char_ptr, "rev_c")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let rev_ws = is_ws!(rev_c);
        let rev_cont = self.builder.build_and(rev_pos, rev_ws, "rev_cont")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(rev_cont, rev_body, trim_out)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(rev_body);
        let ei2 = self.builder.build_int_sub(ei_cur, i64_ty.const_int(1, false), "ei_dec")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(end_alloca, ei2)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(rev_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // Build result string
        self.builder.position_at_end(trim_out);
        let si_out = self.builder.build_load(i64_ty, start_alloca, "si_out")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let ei_out = self.builder.build_load(i64_ty, end_alloca, "ei_out")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let trimmed_len = self.builder.build_int_sub(
            self.builder.build_int_add(ei_out, i64_ty.const_int(1, false), "trim_len1")
                .map_err(|e| CodeGenError::new(e.to_string()))?,
            si_out, "trim_len2"
        ).map_err(|e| CodeGenError::new(e.to_string()))?;
        let is_empty = self.builder.build_int_compare(
            inkwell::IntPredicate::SLE, trimmed_len, i64_ty.const_int(0, false), "trim_empty"
        ).map_err(|e| CodeGenError::new(e.to_string()))?;
        let safe_len = self.builder.build_select(is_empty, i64_ty.const_int(0, false), trimmed_len, "safe_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let alloc_len = self.builder.build_int_add(safe_len, i64_ty.const_int(1, false), "alloc_l")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let buf_call = self.builder.build_call(malloc, &[alloc_len.into()], "trim_buf")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let buf_ptr = match buf_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(src_ptr),
        };
        let src_start = unsafe { self.builder.build_gep(i8_ty, src_ptr, &[si_out], "trim_src")
            .map_err(|e| CodeGenError::new(e.to_string()))? };
        self.builder.build_call(memcpy_fn, &[buf_ptr.into(), src_start.into(), safe_len.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let null_gep = unsafe { self.builder.build_gep(i8_ty, buf_ptr, &[safe_len], "trim_null")
            .map_err(|e| CodeGenError::new(e.to_string()))? };
        self.builder.build_store(null_gep, i8_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        Ok(buf_ptr)
    }

    fn build_str_case_convert(
        &mut self,
        src_ptr : inkwell::values::PointerValue<'ctx>,
        upper   : bool,
    ) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64_ty = self.ctx.i64_type();
        let i8_ty  = self.ctx.i8_type();
        let i32_ty = self.ctx.i32_type();

        let strlen = self.module.get_function("strlen").unwrap();
        let malloc = self.module.get_function("malloc").unwrap();
        let strcpy = self.module.get_function("strcpy").unwrap();
        let conv_fn = if upper {
            self.module.get_function("toupper").unwrap()
        } else {
            self.module.get_function("tolower").unwrap()
        };

        let len_call = self.builder.build_call(strlen, &[src_ptr.into()], "cc_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let len = match len_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => i64_ty.const_int(0, false),
        };
        let len1 = self.builder.build_int_add(len, i64_ty.const_int(1, false), "len1")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let buf_call = self.builder.build_call(malloc, &[len1.into()], "cc_buf")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let buf_ptr = match buf_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(ptr_ty.const_null()),
        };
        self.builder.build_call(strcpy, &[buf_ptr.into(), src_ptr.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let cur_fn = self.cur_fn.unwrap();
        let idx_alloca = self.builder.build_alloca(i64_ty, "cc_idx")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let cond_bb = self.ctx.append_basic_block(cur_fn, "cc.cond");
        let body_bb = self.ctx.append_basic_block(cur_fn, "cc.body");
        let exit_bb = self.ctx.append_basic_block(cur_fn, "cc.exit");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(cond_bb);

        let idx = self.builder.build_load(i64_ty, idx_alloca, "cc_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let ch_ptr = unsafe {
            self.builder.build_gep(i8_ty, buf_ptr, &[idx], "cc_chp")
                .map_err(|e| CodeGenError::new(e.to_string()))?
        };
        let ch = self.builder.build_load(i8_ty, ch_ptr, "cc_ch")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let ch_i64 = self.builder.build_int_z_extend(ch, i64_ty, "cc_ch64")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let not_null = self.builder.build_int_compare(
            inkwell::IntPredicate::NE, ch_i64, i64_ty.const_int(0, false), "cc_nz"
        ).map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(not_null, body_bb, exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(body_bb);
        let ch_i32 = self.builder.build_int_z_extend(ch, i32_ty, "cc_i32")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let conv_r = self.builder.build_call(conv_fn, &[ch_i32.into()], "cc_conv")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        if let Some(BasicValueEnum::IntValue(conv_i32)) = conv_r.try_as_basic_value().basic() {
            let conv_i8 = self.builder.build_int_truncate(conv_i32, i8_ty, "cc_i8")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            self.builder.build_store(ch_ptr, conv_i8)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }
        let next_idx = self.builder.build_int_add(idx, i64_ty.const_int(1, false), "cc_inc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, next_idx)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(exit_bb);
        Ok(buf_ptr)
    }

    fn build_str_split(
        &mut self,
        src_ptr : inkwell::values::PointerValue<'ctx>,
        delim   : inkwell::values::PointerValue<'ctx>,
    ) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64_ty = self.ctx.i64_type();

        let list_new = self.module.get_function("arc_list_new").unwrap();
        let list_call = self.builder.build_call(list_new, &[], "split_list")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let list_ptr = match list_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(ptr_ty.const_null()),
        };

        let strlen = self.module.get_function("strlen").unwrap();
        let malloc = self.module.get_function("malloc").unwrap();
        let strcpy = self.module.get_function("strcpy").unwrap();
        let len_r  = self.builder.build_call(strlen, &[src_ptr.into()], "sp_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let len = match len_r.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => return Ok(list_ptr),
        };
        let len1 = self.builder.build_int_add(len, i64_ty.const_int(1, false), "sp_len1")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let copy_call = self.builder.build_call(malloc, &[len1.into()], "sp_copy")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let copy_ptr = match copy_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(list_ptr),
        };
        self.builder.build_call(strcpy, &[copy_ptr.into(), src_ptr.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let strtok = self.module.get_function("strtok").unwrap();
        let token_alloca = self.builder.build_alloca(ptr_ty, "sp_tok")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let first_call = self.builder.build_call(strtok, &[copy_ptr.into(), delim.into()], "sp_first")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let first_tok = match first_call.try_as_basic_value().basic() {
            Some(v) => v,
            None => return Ok(list_ptr),
        };
        self.builder.build_store(token_alloca, first_tok)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let cur_fn   = self.cur_fn.unwrap();
        let cond_bb  = self.ctx.append_basic_block(cur_fn, "sp.cond");
        let body_bb  = self.ctx.append_basic_block(cur_fn, "sp.body");
        let exit_bb  = self.ctx.append_basic_block(cur_fn, "sp.exit");
        let list_append = self.module.get_function("arc_list_append").unwrap();
        let null_ptr = ptr_ty.const_null();

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(cond_bb);

        let tok = self.builder.build_load(ptr_ty, token_alloca, "sp_tok_v")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let tok_int = self.builder.build_ptr_to_int(
            tok.into_pointer_value(), i64_ty, "tok_i"
        ).map_err(|e| CodeGenError::new(e.to_string()))?;
        let null_int = self.builder.build_ptr_to_int(null_ptr, i64_ty, "null_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let not_null = self.builder.build_int_compare(
            inkwell::IntPredicate::NE, tok_int, null_int, "sp_nz"
        ).map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(not_null, body_bb, exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(body_bb);
        let tok_v = self.builder.build_load(ptr_ty, token_alloca, "sp_tok_v2")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_pointer_value();
        let tok_i64 = self.builder.build_ptr_to_int(tok_v, i64_ty, "tok_i64")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_call(list_append, &[list_ptr.into(), tok_i64.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let next_call = self.builder.build_call(strtok, &[null_ptr.into(), delim.into()], "sp_next")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let next_tok = match next_call.try_as_basic_value().basic() {
            Some(v) => v,
            None => ptr_ty.const_null().into(),
        };
        self.builder.build_store(token_alloca, next_tok)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(exit_bb);
        Ok(list_ptr)
    }

    fn pre_declare_arc_runtime_fns(&mut self) {
        let ptr_ty  = self.ctx.ptr_type(AddressSpace::default());
        let i64_ty  = self.ctx.i64_type();
        let void_ty = self.ctx.void_type();

        let decls: &[(&str, bool, &[inkwell::types::BasicMetadataTypeEnum], bool)] = &[
        ];
        let _ = decls;

        macro_rules! decl {
            ($name:expr, ptr, [$($p:expr),*]) => {
                if self.module.get_function($name).is_none() {
                    let ps: Vec<inkwell::types::BasicMetadataTypeEnum> = vec![$($p),*];
                    self.module.add_function($name, ptr_ty.fn_type(&ps, false), None);
                }
            };
            ($name:expr, i64, [$($p:expr),*]) => {
                if self.module.get_function($name).is_none() {
                    let ps: Vec<inkwell::types::BasicMetadataTypeEnum> = vec![$($p),*];
                    self.module.add_function($name, i64_ty.fn_type(&ps, false), None);
                }
            };
            ($name:expr, void, [$($p:expr),*]) => {
                if self.module.get_function($name).is_none() {
                    let ps: Vec<inkwell::types::BasicMetadataTypeEnum> = vec![$($p),*];
                    self.module.add_function($name, void_ty.fn_type(&ps, false), None);
                }
            };
        }

        decl!("arc_list_new",    ptr,  []);
        decl!("arc_list_append", void, [ptr_ty.into(), i64_ty.into()]);
        decl!("arc_list_length", i64,  [ptr_ty.into()]);
        decl!("arc_list_get",    i64,  [ptr_ty.into(), i64_ty.into()]);
        decl!("arc_list_set",    void, [ptr_ty.into(), i64_ty.into(), i64_ty.into()]);
        decl!("arc_list_filter", ptr,  [ptr_ty.into(), ptr_ty.into()]);
        decl!("arc_map_new",             ptr,  []);
        decl!("arc_map_set",             void, [ptr_ty.into(), ptr_ty.into(), i64_ty.into()]);
        decl!("arc_map_get_or_default",  i64,  [ptr_ty.into(), ptr_ty.into(), i64_ty.into()]);
        decl!("arc_pair_new",    ptr,  [i64_ty.into(), i64_ty.into()]);
        decl!("arc_pair_first",  i64,  [ptr_ty.into()]);
        decl!("arc_pair_second", i64,  [ptr_ty.into()]);
    }

    // Layout: header = {length:i64, capacity:i64, data_ptr_as_i64:i64}
    // data = separately malloc'd i64 array

    fn gen_arc_list_new(&mut self) {
        let fn_val = match self.module.get_function("arc_list_new") {
            Some(f) if f.count_basic_blocks() > 0 => return,
            Some(f) => f,
            None => return,
        };
        let i64_ty = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let malloc  = self.module.get_function("malloc").unwrap();
        let entry = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);
        // Alloc header: 3 * 8 = 24 bytes
        let hdr_sz = i64_ty.const_int(24, false);
        let hdr = self.builder.build_call(malloc, &[hdr_sz.into()], "hdr")
            .unwrap().try_as_basic_value().basic().unwrap().into_pointer_value();
        // Alloc initial data: 8 * 8 = 64 bytes (capacity=8)
        let init_cap: u64 = 8;
        let data_sz = i64_ty.const_int(init_cap * 8, false);
        let data = self.builder.build_call(malloc, &[data_sz.into()], "data")
            .unwrap().try_as_basic_value().basic().unwrap().into_pointer_value();
        // header[0] = 0 (length)
        let slot0 = unsafe { self.builder.build_gep(i64_ty, hdr, &[i64_ty.const_int(0,false)], "s0").unwrap() };
        self.builder.build_store(slot0, i64_ty.const_int(0, false)).unwrap();
        // header[1] = init_cap (capacity)
        let slot1 = unsafe { self.builder.build_gep(i64_ty, hdr, &[i64_ty.const_int(1,false)], "s1").unwrap() };
        self.builder.build_store(slot1, i64_ty.const_int(init_cap, false)).unwrap();
        // header[2] = ptrtoint(data)
        let slot2 = unsafe { self.builder.build_gep(i64_ty, hdr, &[i64_ty.const_int(2,false)], "s2").unwrap() };
        let data_int = self.builder.build_ptr_to_int(data, i64_ty, "di").unwrap();
        self.builder.build_store(slot2, data_int).unwrap();
        let _ = ptr_ty;
        self.builder.build_return(Some(&hdr)).unwrap();
    }

    fn gen_arc_list_append(&mut self) {
        let fn_val = match self.module.get_function("arc_list_append") {
            Some(f) if f.count_basic_blocks() > 0 => return,
            Some(f) => f,
            None => return,
        };
        let i64_ty = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let malloc  = self.module.get_function("malloc").unwrap();
        // Get memcpy
        let memcpy_fn = if let Some(f) = self.module.get_function("memcpy") { f } else {
            let ft = ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), i64_ty.into()], false);
            self.module.add_function("memcpy", ft, None)
        };

        let entry_bb = self.ctx.append_basic_block(fn_val, "entry");
        let grow_bb  = self.ctx.append_basic_block(fn_val, "grow");
        let store_bb = self.ctx.append_basic_block(fn_val, "store");

        self.builder.position_at_end(entry_bb);
        let list_ptr = fn_val.get_nth_param(0).unwrap().into_pointer_value();
        let item     = fn_val.get_nth_param(1).unwrap().into_int_value();

        let slot0 = unsafe { self.builder.build_gep(i64_ty, list_ptr, &[i64_ty.const_int(0,false)], "s0").unwrap() };
        let slot1 = unsafe { self.builder.build_gep(i64_ty, list_ptr, &[i64_ty.const_int(1,false)], "s1").unwrap() };

        let len = self.builder.build_load(inkwell::types::BasicTypeEnum::IntType(i64_ty), slot0, "len").unwrap().into_int_value();
        let cap = self.builder.build_load(inkwell::types::BasicTypeEnum::IntType(i64_ty), slot1, "cap").unwrap().into_int_value();
        let full = self.builder.build_int_compare(inkwell::IntPredicate::SGE, len, cap, "full").unwrap();
        self.builder.build_conditional_branch(full, grow_bb, store_bb).unwrap();

        // GROW block
        self.builder.position_at_end(grow_bb);
        let len_g = self.builder.build_load(inkwell::types::BasicTypeEnum::IntType(i64_ty), slot0, "len_g").unwrap().into_int_value();
        let cap_g = self.builder.build_load(inkwell::types::BasicTypeEnum::IntType(i64_ty), slot1, "cap_g").unwrap().into_int_value();
        let new_cap = self.builder.build_int_mul(cap_g, i64_ty.const_int(2, false), "nc").unwrap();
        let new_sz  = self.builder.build_int_mul(new_cap, i64_ty.const_int(8, false), "ns").unwrap();
        let new_data = self.builder.build_call(malloc, &[new_sz.into()], "nd")
            .unwrap().try_as_basic_value().basic().unwrap().into_pointer_value();
        let old_dp = { let ptr_ty2 = self.ctx.ptr_type(AddressSpace::default()); let s2x = unsafe { self.builder.build_gep(i64_ty, list_ptr, &[i64_ty.const_int(2,false)], "s2x").unwrap() }; let di2 = self.builder.build_load(inkwell::types::BasicTypeEnum::IntType(i64_ty), s2x, "di2").unwrap().into_int_value(); self.builder.build_int_to_ptr(di2, ptr_ty2, "odp").unwrap() };
        let old_sz = self.builder.build_int_mul(len_g, i64_ty.const_int(8, false), "os").unwrap();
        self.builder.build_call(memcpy_fn, &[new_data.into(), old_dp.into(), old_sz.into()], "").unwrap();
        self.builder.build_store(slot1, new_cap).unwrap();
        let slot2g = unsafe { self.builder.build_gep(i64_ty, list_ptr, &[i64_ty.const_int(2,false)], "s2g").unwrap() };
        let nd_int = self.builder.build_ptr_to_int(new_data, i64_ty, "ndi").unwrap();
        self.builder.build_store(slot2g, nd_int).unwrap();
        self.builder.build_unconditional_branch(store_bb).unwrap();

        // STORE block
        self.builder.position_at_end(store_bb);
        let len_s = self.builder.build_load(inkwell::types::BasicTypeEnum::IntType(i64_ty), slot0, "len_s").unwrap().into_int_value();
        let data_ptr = { let ptr_ty2 = self.ctx.ptr_type(AddressSpace::default()); let s2x = unsafe { self.builder.build_gep(i64_ty, list_ptr, &[i64_ty.const_int(2,false)], "s2x").unwrap() }; let di2 = self.builder.build_load(inkwell::types::BasicTypeEnum::IntType(i64_ty), s2x, "di2").unwrap().into_int_value(); self.builder.build_int_to_ptr(di2, ptr_ty2, "dp").unwrap() };
        let ep = unsafe { self.builder.build_gep(i64_ty, data_ptr, &[len_s], "ep").unwrap() };
        self.builder.build_store(ep, item).unwrap();
        let nl = self.builder.build_int_add(len_s, i64_ty.const_int(1, false), "nl").unwrap();
        self.builder.build_store(slot0, nl).unwrap();
        let _ = ptr_ty;
        self.builder.build_return(None).unwrap();
    }

    fn gen_arc_list_length(&mut self) {
        let fn_val = match self.module.get_function("arc_list_length") {
            Some(f) if f.count_basic_blocks() > 0 => return,
            Some(f) => f,
            None => return,
        };
        let i64_ty = self.ctx.i64_type();
        let entry = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);
        let list_ptr = fn_val.get_nth_param(0).unwrap().into_pointer_value();
        let slot0 = unsafe { self.builder.build_gep(i64_ty, list_ptr, &[i64_ty.const_int(0,false)], "s0").unwrap() };
        let len = self.builder.build_load(inkwell::types::BasicTypeEnum::IntType(i64_ty), slot0, "len").unwrap();
        self.builder.build_return(Some(&len)).unwrap();
    }

    fn gen_arc_list_get(&mut self) {
        let fn_val = match self.module.get_function("arc_list_get") {
            Some(f) if f.count_basic_blocks() > 0 => return,
            Some(f) => f,
            None => return,
        };
        let i64_ty = self.ctx.i64_type();
        let entry = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);
        let list_ptr = fn_val.get_nth_param(0).unwrap().into_pointer_value();
        let idx = fn_val.get_nth_param(1).unwrap().into_int_value();
        let data_ptr = { let ptr_ty2 = self.ctx.ptr_type(AddressSpace::default()); let s2x = unsafe { self.builder.build_gep(i64_ty, list_ptr, &[i64_ty.const_int(2,false)], "s2x").unwrap() }; let di2 = self.builder.build_load(inkwell::types::BasicTypeEnum::IntType(i64_ty), s2x, "di2").unwrap().into_int_value(); self.builder.build_int_to_ptr(di2, ptr_ty2, "dp").unwrap() };
        let ep = unsafe { self.builder.build_gep(i64_ty, data_ptr, &[idx], "ep").unwrap() };
        let val = self.builder.build_load(inkwell::types::BasicTypeEnum::IntType(i64_ty), ep, "val").unwrap();
        self.builder.build_return(Some(&val)).unwrap();
    }

    fn gen_arc_list_set(&mut self) {
        let fn_name = "arc_list_set";
        if let Some(f) = self.module.get_function(fn_name) {
            if f.count_basic_blocks() > 0 { return; }
        } else {
            let i64_ty = self.ctx.i64_type();
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let fn_ty  = self.ctx.void_type().fn_type(
                &[ptr_ty.into(), i64_ty.into(), i64_ty.into()], false);
            self.module.add_function(fn_name, fn_ty, None);
        }
        let fn_val = self.module.get_function(fn_name).unwrap();
        if fn_val.count_basic_blocks() > 0 { return; }
        let i64_ty = self.ctx.i64_type();
        let entry  = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);
        let list_ptr = fn_val.get_nth_param(0).unwrap().into_pointer_value();
        let idx      = fn_val.get_nth_param(1).unwrap().into_int_value();
        let val      = fn_val.get_nth_param(2).unwrap().into_int_value();
        let data_ptr = { let ptr_ty2 = self.ctx.ptr_type(AddressSpace::default()); let s2x = unsafe { self.builder.build_gep(i64_ty, list_ptr, &[i64_ty.const_int(2,false)], "s2x").unwrap() }; let di2 = self.builder.build_load(inkwell::types::BasicTypeEnum::IntType(i64_ty), s2x, "di2").unwrap().into_int_value(); self.builder.build_int_to_ptr(di2, ptr_ty2, "dp").unwrap() };
        let ep = unsafe { self.builder.build_gep(i64_ty, data_ptr, &[idx], "ep").unwrap() };
        self.builder.build_store(ep, val).unwrap();
        self.builder.build_return(None).unwrap();
    }

    fn gen_arc_list_filter(&mut self) {
        let fn_val = match self.module.get_function("arc_list_filter") {
            Some(f) if f.count_basic_blocks() > 0 => return,
            Some(f) => f,
            None => return,
        };
        let i64_ty = self.ctx.i64_type();

        let entry_bb  = self.ctx.append_basic_block(fn_val, "entry");
        let cond_bb   = self.ctx.append_basic_block(fn_val, "filter.cond");
        let body_bb   = self.ctx.append_basic_block(fn_val, "filter.body");
        let append_bb = self.ctx.append_basic_block(fn_val, "filter.append");
        let next_bb   = self.ctx.append_basic_block(fn_val, "filter.next");
        let exit_bb   = self.ctx.append_basic_block(fn_val, "filter.exit");

        self.builder.position_at_end(entry_bb);
        let list_ptr = fn_val.get_nth_param(0).unwrap().into_pointer_value();
        let fn_ptr   = fn_val.get_nth_param(1).unwrap().into_pointer_value();

        let new_fn   = self.module.get_function("arc_list_new").unwrap();
        let len_fn   = self.module.get_function("arc_list_length").unwrap();
        let get_fn   = self.module.get_function("arc_list_get").unwrap();
        let app_fn   = self.module.get_function("arc_list_append").unwrap();

        let out = self.builder.build_call(new_fn, &[], "out")
            .unwrap().try_as_basic_value().basic().unwrap();
        let len_v = self.builder.build_call(len_fn, &[list_ptr.into()], "len")
            .unwrap().try_as_basic_value().basic().unwrap().into_int_value();

        let idx_slot = self.builder.build_alloca(i64_ty, "idx_slot").unwrap();
        self.builder.build_store(idx_slot, i64_ty.const_int(0, false)).unwrap();
        self.builder.build_unconditional_branch(cond_bb).unwrap();

        self.builder.position_at_end(cond_bb);
        let idx = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), idx_slot, "idx"
        ).unwrap().into_int_value();
        let cmp = self.builder.build_int_compare(
            inkwell::IntPredicate::SLT, idx, len_v, "cmp"
        ).unwrap();
        self.builder.build_conditional_branch(cmp, body_bb, exit_bb).unwrap();

        self.builder.position_at_end(body_bb);
        let idx2 = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), idx_slot, "idx2"
        ).unwrap().into_int_value();
        let item = self.builder.build_call(get_fn, &[list_ptr.into(), idx2.into()], "item")
            .unwrap().try_as_basic_value().basic().unwrap().into_int_value();

        let fn_ty = i64_ty.fn_type(&[i64_ty.into()], false);
        let res = self.builder.build_indirect_call(fn_ty, fn_ptr, &[item.into()], "res")
            .unwrap().try_as_basic_value().basic().unwrap().into_int_value();
        let is_true = self.builder.build_int_compare(
            inkwell::IntPredicate::NE, res, i64_ty.const_int(0, false), "is_true"
        ).unwrap();
        self.builder.build_conditional_branch(is_true, append_bb, next_bb).unwrap();

        self.builder.position_at_end(append_bb);
        let out_ptr = out.into_pointer_value();
        self.builder.build_call(app_fn, &[out_ptr.into(), item.into()], "").unwrap();
        self.builder.build_unconditional_branch(next_bb).unwrap();

        self.builder.position_at_end(next_bb);
        let idx3 = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), idx_slot, "idx3"
        ).unwrap().into_int_value();
        let idx4 = self.builder.build_int_add(idx3, i64_ty.const_int(1, false), "idx4").unwrap();
        self.builder.build_store(idx_slot, idx4).unwrap();
        self.builder.build_unconditional_branch(cond_bb).unwrap();

        self.builder.position_at_end(exit_bb);
        self.builder.build_return(Some(&out)).unwrap();
    }

    fn gen_arc_map_new(&mut self) {
        let fn_val = match self.module.get_function("arc_map_new") {
            Some(f) if f.count_basic_blocks() > 0 => return,
            Some(f) => f,
            None => return,
        };
        let i64_ty = self.ctx.i64_type();
        let malloc  = self.module.get_function("malloc").unwrap();

        let entry = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);

        let sz = i64_ty.const_int(8 * 129, false);
        let ptr = self.builder.build_call(malloc, &[sz.into()], "map_ptr")
            .unwrap().try_as_basic_value().basic().unwrap();
        if let inkwell::values::BasicValueEnum::PointerValue(p) = ptr {
            self.builder.build_store(p, i64_ty.const_int(0, false)).unwrap();
            self.builder.build_return(Some(&p)).unwrap();
        }
    }

    fn gen_arc_map_set(&mut self) {
        let fn_val = match self.module.get_function("arc_map_set") {
            Some(f) if f.count_basic_blocks() > 0 => return,
            Some(f) => f,
            None => return,
        };
        let i64_ty  = self.ctx.i64_type();
        let strcmp  = self.module.get_function("strcmp").unwrap();

        let entry_bb  = self.ctx.append_basic_block(fn_val, "entry");
        let cond_bb   = self.ctx.append_basic_block(fn_val, "set.cond");
        let check_bb  = self.ctx.append_basic_block(fn_val, "set.check");
        let update_bb = self.ctx.append_basic_block(fn_val, "set.update");
        let next_bb   = self.ctx.append_basic_block(fn_val, "set.next");
        let insert_bb = self.ctx.append_basic_block(fn_val, "set.insert");

        self.builder.position_at_end(entry_bb);
        let map_ptr = fn_val.get_nth_param(0).unwrap().into_pointer_value();
        let key_ptr = fn_val.get_nth_param(1).unwrap().into_pointer_value();
        let val_v   = fn_val.get_nth_param(2).unwrap().into_int_value();

        let len = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), map_ptr, "len"
        ).unwrap().into_int_value();

        let i_slot = self.builder.build_alloca(i64_ty, "i").unwrap();
        self.builder.build_store(i_slot, i64_ty.const_int(0, false)).unwrap();
        self.builder.build_unconditional_branch(cond_bb).unwrap();

        self.builder.position_at_end(cond_bb);
        let i = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), i_slot, "i"
        ).unwrap().into_int_value();
        let lt = self.builder.build_int_compare(
            inkwell::IntPredicate::SLT, i, len, "lt"
        ).unwrap();
        self.builder.build_conditional_branch(lt, check_bb, insert_bb).unwrap();

        self.builder.position_at_end(check_bb);
        let i2 = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), i_slot, "i2"
        ).unwrap().into_int_value();
        let two    = i64_ty.const_int(2, false);
        let one    = i64_ty.const_int(1, false);
        let i2t    = self.builder.build_int_mul(i2, two, "i2t").unwrap();
        let ki     = self.builder.build_int_add(i2t, one, "ki").unwrap();
        let kslot  = unsafe {
            self.builder.build_gep(i64_ty, map_ptr, &[ki], "kslot").unwrap()
        };
        let stored_key_i = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), kslot, "ski"
        ).unwrap().into_int_value();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let stored_key_ptr = self.builder.build_int_to_ptr(stored_key_i, ptr_ty, "skp").unwrap();

        let cmp_res = self.builder.build_call(
            strcmp, &[stored_key_ptr.into(), key_ptr.into()], "cmp"
        ).unwrap().try_as_basic_value().basic().unwrap().into_int_value();
        let cmp_zero = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ, cmp_res, self.ctx.i32_type().const_int(0, false), "eq"
        ).unwrap();
        self.builder.build_conditional_branch(cmp_zero, update_bb, next_bb).unwrap();

        self.builder.position_at_end(update_bb);
        let i3 = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), i_slot, "i3"
        ).unwrap().into_int_value();
        let i3t   = self.builder.build_int_mul(i3, two, "i3t").unwrap();
        let vi    = self.builder.build_int_add(i3t, i64_ty.const_int(2, false), "vi").unwrap();
        let vslot = unsafe {
            self.builder.build_gep(i64_ty, map_ptr, &[vi], "vs").unwrap()
        };
        self.builder.build_store(vslot, val_v).unwrap();
        self.builder.build_return(None).unwrap();

        self.builder.position_at_end(next_bb);
        let i4 = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), i_slot, "i4"
        ).unwrap().into_int_value();
        let i5 = self.builder.build_int_add(i4, one, "i5").unwrap();
        self.builder.build_store(i_slot, i5).unwrap();
        self.builder.build_unconditional_branch(cond_bb).unwrap();

        self.builder.position_at_end(insert_bb);
        let len2 = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), map_ptr, "len2"
        ).unwrap().into_int_value();
        let l2t  = self.builder.build_int_mul(len2, two, "l2t").unwrap();
        let ki2  = self.builder.build_int_add(l2t, one, "ki2").unwrap();
        let vi2  = self.builder.build_int_add(l2t, i64_ty.const_int(2, false), "vi2").unwrap();

        let key_as_i = self.builder.build_ptr_to_int(key_ptr, i64_ty, "kasi").unwrap();
        let kslot2 = unsafe {
            self.builder.build_gep(i64_ty, map_ptr, &[ki2], "ksl2").unwrap()
        };
        self.builder.build_store(kslot2, key_as_i).unwrap();

        let vslot2 = unsafe {
            self.builder.build_gep(i64_ty, map_ptr, &[vi2], "vsl2").unwrap()
        };
        self.builder.build_store(vslot2, val_v).unwrap();

        let new_len = self.builder.build_int_add(len2, one, "nl").unwrap();
        self.builder.build_store(map_ptr, new_len).unwrap();
        self.builder.build_return(None).unwrap();
    }

    fn gen_arc_map_get_or_default(&mut self) {
        let fn_val = match self.module.get_function("arc_map_get_or_default") {
            Some(f) if f.count_basic_blocks() > 0 => return,
            Some(f) => f,
            None => return,
        };
        let i64_ty = self.ctx.i64_type();
        let strcmp = self.module.get_function("strcmp").unwrap();

        let entry_bb  = self.ctx.append_basic_block(fn_val, "entry");
        let cond_bb   = self.ctx.append_basic_block(fn_val, "get.cond");
        let check_bb  = self.ctx.append_basic_block(fn_val, "get.check");
        let found_bb  = self.ctx.append_basic_block(fn_val, "get.found");
        let next_bb   = self.ctx.append_basic_block(fn_val, "get.next");
        let miss_bb   = self.ctx.append_basic_block(fn_val, "get.miss");

        self.builder.position_at_end(entry_bb);
        let map_ptr = fn_val.get_nth_param(0).unwrap().into_pointer_value();
        let key_ptr = fn_val.get_nth_param(1).unwrap().into_pointer_value();
        let def_v   = fn_val.get_nth_param(2).unwrap().into_int_value();

        let len = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), map_ptr, "len"
        ).unwrap().into_int_value();

        let i_slot = self.builder.build_alloca(i64_ty, "i").unwrap();
        self.builder.build_store(i_slot, i64_ty.const_int(0, false)).unwrap();
        self.builder.build_unconditional_branch(cond_bb).unwrap();

        self.builder.position_at_end(cond_bb);
        let i = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), i_slot, "i"
        ).unwrap().into_int_value();
        let lt = self.builder.build_int_compare(
            inkwell::IntPredicate::SLT, i, len, "lt"
        ).unwrap();
        self.builder.build_conditional_branch(lt, check_bb, miss_bb).unwrap();

        self.builder.position_at_end(check_bb);
        let ic = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), i_slot, "ic"
        ).unwrap().into_int_value();
        let two = i64_ty.const_int(2, false);
        let one = i64_ty.const_int(1, false);
        let ict  = self.builder.build_int_mul(ic, two, "ict").unwrap();
        let ki   = self.builder.build_int_add(ict, one, "ki").unwrap();
        let kslot = unsafe {
            self.builder.build_gep(i64_ty, map_ptr, &[ki], "ks").unwrap()
        };
        let ski = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), kslot, "ski"
        ).unwrap().into_int_value();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let skp = self.builder.build_int_to_ptr(ski, ptr_ty, "skp").unwrap();
        let cmp = self.builder.build_call(
            strcmp, &[skp.into(), key_ptr.into()], "cmp"
        ).unwrap().try_as_basic_value().basic().unwrap().into_int_value();
        let eq = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ, cmp, self.ctx.i32_type().const_int(0, false), "eq"
        ).unwrap();
        self.builder.build_conditional_branch(eq, found_bb, next_bb).unwrap();

        self.builder.position_at_end(found_bb);
        let if2 = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), i_slot, "if2"
        ).unwrap().into_int_value();
        let if2t = self.builder.build_int_mul(if2, two, "if2t").unwrap();
        let vi   = self.builder.build_int_add(if2t, i64_ty.const_int(2, false), "vi").unwrap();
        let vslot = unsafe {
            self.builder.build_gep(i64_ty, map_ptr, &[vi], "vs").unwrap()
        };
        let val = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), vslot, "val"
        ).unwrap();
        self.builder.build_return(Some(&val)).unwrap();

        self.builder.position_at_end(next_bb);
        let in_ = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), i_slot, "in"
        ).unwrap().into_int_value();
        let in2 = self.builder.build_int_add(in_, one, "in2").unwrap();
        self.builder.build_store(i_slot, in2).unwrap();
        self.builder.build_unconditional_branch(cond_bb).unwrap();

        self.builder.position_at_end(miss_bb);
        self.builder.build_return(Some(&def_v)).unwrap();
    }

    fn gen_arc_pair_new(&mut self) {
        let fn_val = match self.module.get_function("arc_pair_new") {
            Some(f) if f.count_basic_blocks() > 0 => return,
            Some(f) => f,
            None => return,
        };
        let i64_ty = self.ctx.i64_type();
        let malloc  = self.module.get_function("malloc").unwrap();

        let entry = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);

        let fst = fn_val.get_nth_param(0).unwrap().into_int_value();
        let snd = fn_val.get_nth_param(1).unwrap().into_int_value();

        let sz = i64_ty.const_int(16, false);
        let p  = self.builder.build_call(malloc, &[sz.into()], "pair_ptr")
            .unwrap().try_as_basic_value().basic().unwrap();

        if let inkwell::values::BasicValueEnum::PointerValue(ptr) = p {
            self.builder.build_store(ptr, fst).unwrap();
            let snd_ptr = unsafe {
                self.builder.build_gep(i64_ty, ptr, &[i64_ty.const_int(1, false)], "snd_ptr").unwrap()
            };
            self.builder.build_store(snd_ptr, snd).unwrap();
            self.builder.build_return(Some(&ptr)).unwrap();
        }
    }

    fn gen_arc_pair_first(&mut self) {
        let fn_val = match self.module.get_function("arc_pair_first") {
            Some(f) if f.count_basic_blocks() > 0 => return,
            Some(f) => f,
            None => return,
        };
        let i64_ty = self.ctx.i64_type();
        let entry = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);

        let pair_ptr = fn_val.get_nth_param(0).unwrap().into_pointer_value();
        let val = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), pair_ptr, "fst"
        ).unwrap();
        self.builder.build_return(Some(&val)).unwrap();
    }

    fn gen_arc_pair_second(&mut self) {
        let fn_val = match self.module.get_function("arc_pair_second") {
            Some(f) if f.count_basic_blocks() > 0 => return,
            Some(f) => f,
            None => return,
        };
        let i64_ty = self.ctx.i64_type();
        let entry = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);

        let pair_ptr = fn_val.get_nth_param(0).unwrap().into_pointer_value();
        let snd_ptr = unsafe {
            self.builder.build_gep(i64_ty, pair_ptr, &[i64_ty.const_int(1, false)], "sp").unwrap()
        };
        let val = self.builder.build_load(
            inkwell::types::BasicTypeEnum::IntType(i64_ty), snd_ptr, "snd"
        ).unwrap();
        self.builder.build_return(Some(&val)).unwrap();
    }

    fn declare_math_fns(&mut self) {
        let f64_ty = self.ctx.f64_type();
        for (name, nargs) in &[("sqrt", 1usize), ("fabs", 1), ("pow", 2)] {
            if self.module.get_function(name).is_some() { continue; }
            let params: Vec<inkwell::types::BasicMetadataTypeEnum> =
                (0..*nargs).map(|_| f64_ty.into()).collect();
            let ft = f64_ty.fn_type(&params, false);
            self.module.add_function(name, ft, None);
        }
    }

    fn declare_arc_runtime(&mut self) {
        if self.module.get_function("arc_generate_id").is_some() { return; }

        if self.module.get_function("sprintf").is_none() {
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let i32_ty = self.ctx.i32_type();
            let ft = i32_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], true);
            self.module.add_function("sprintf", ft, None);
        }

        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64_ty = self.ctx.i64_type();

        let counter = self.module.add_global(i64_ty, None, "arc_id_counter");
        counter.set_initializer(&i64_ty.const_int(0, false));
        counter.set_linkage(inkwell::module::Linkage::Internal);

        let buf_ty = self.ctx.i8_type().array_type(32);
        let buf = self.module.add_global(buf_ty, None, "arc_id_buf");
        buf.set_initializer(&buf_ty.const_zero());
        buf.set_linkage(inkwell::module::Linkage::Internal);

        let fmt_bytes = b"id-%lld\0";
        let fmt_arr = self.ctx.const_string(fmt_bytes, false);
        let fmt_global = self.module.add_global(fmt_arr.get_type(), None, "arc_id_fmt");
        fmt_global.set_initializer(&fmt_arr);
        fmt_global.set_linkage(inkwell::module::Linkage::Internal);

        let ft = ptr_ty.fn_type(&[], false);
        let fn_val = self.module.add_function("arc_generate_id", ft, None);
        let entry = self.ctx.append_basic_block(fn_val, "entry");

        let prev_block = self.builder.get_insert_block();
        self.builder.position_at_end(entry);

        let n = self.builder.build_load(BasicTypeEnum::IntType(i64_ty), counter.as_pointer_value(), "n")
            .unwrap().into_int_value();
        let n1 = self.builder.build_int_add(n, i64_ty.const_int(1, false), "n1").unwrap();
        self.builder.build_store(counter.as_pointer_value(), n1).unwrap();

        let buf_ptr = buf.as_pointer_value();

        let sprintf = self.module.get_function("sprintf").unwrap();
        let fmt_ptr = fmt_global.as_pointer_value();
        self.builder.build_call(sprintf, &[buf_ptr.into(), fmt_ptr.into(), n.into()], "").unwrap();

        self.builder.build_return(Some(&buf_ptr)).unwrap();

        if let Some(bb) = prev_block {
            self.builder.position_at_end(bb);
        }
    }

    fn to_f64(&mut self, val: BasicValueEnum<'ctx>)
        -> CgResult<inkwell::values::FloatValue<'ctx>>
    {
        let f64ty = self.ctx.f64_type();
        match val {
            BasicValueEnum::FloatValue(f) => {
                if f.get_type().get_bit_width() < 64 {
                    self.builder.build_float_ext(f, f64ty, "fpext64")
                        .map_err(|e| CodeGenError::new(e.to_string()))
                } else {
                    Ok(f)
                }
            }
            BasicValueEnum::IntValue(i) => {
                self.builder.build_signed_int_to_float(i, f64ty, "itof")
                    .map_err(|e| CodeGenError::new(e.to_string()))
            }
            _ => Err(CodeGenError::new("cannot convert to f64")),
        }
    }

    fn compile_io_print(&mut self, args: &[Expr]) -> CgResult<()> {
        let printf = self.module.get_function("printf")
            .ok_or_else(|| CodeGenError::new("printf not declared"))?;

        if args.is_empty() {
            return Ok(());
        }

        match &args[0] {
            Expr::StrLit(s) => {
                let fmt = self.build_global_string(s)?;
                self.builder.build_call(printf, &[fmt.into()], "print")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
            }

            Expr::StrInterp(parts) => {
                let mut fmt_str   = String::new();
                let mut interp_vals: Vec<BasicValueEnum<'ctx>> = Vec::new();

                for part in parts {
                    match part {
                        StringPart::Text(t) => {
                            fmt_str.push_str(&t.replace('%', "%%"));
                        }
                        StringPart::Interp(inner_expr) => {
                            if let Some(ep) = self.try_enum_label(inner_expr) {
                                fmt_str.push_str("%s");
                                interp_vals.push(ep.into());
                            } else if let Some(val) = self.compile_expr(inner_expr)? {
                                // Boolean i1 → "true"/"false"
                                if let BasicValueEnum::IntValue(iv) = val {
                                    if iv.get_type().get_bit_width() == 1 {
                                        let ts = self.build_global_string("true")?;
                                        let fs = self.build_global_string("false")?;
                                        let sel = self.builder.build_select(iv, ts, fs, "bool_str")
                                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                                        fmt_str.push_str("%s");
                                        interp_vals.push(sel.into());
                                        continue;
                                    }
                                }
                                let spec = match val {
                                    BasicValueEnum::IntValue(iv) => {
                                        match iv.get_type().get_bit_width() {
                                            64 => "%lld",
                                            _  => "%d",
                                        }
                                    }
                                    BasicValueEnum::FloatValue(_) => "%g",
                                    BasicValueEnum::PointerValue(_) => "%s",
                                    _ => "%d",
                                };
                                fmt_str.push_str(spec);
                                let promoted = match val {
                                    BasicValueEnum::FloatValue(f) => {
                                        let f64ty = self.ctx.f64_type();
                                        if f.get_type().get_bit_width() < 64 {
                                            self.builder.build_float_ext(f, f64ty, "fpext")
                                                .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                                        } else {
                                            val
                                        }
                                    }
                                    BasicValueEnum::IntValue(iv) if iv.get_type().get_bit_width() < 32 => {
                                        let i32ty = self.ctx.i32_type();
                                        self.builder.build_int_z_extend(iv, i32ty, "zext")
                                            .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                                    }
                                    _ => val,
                                };
                                interp_vals.push(promoted);
                            } else {
                                fmt_str.push_str("(null)");
                            }
                        }
                    }
                }
                let fmt_ptr = self.build_global_string(&fmt_str)?;
                let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum> = vec![fmt_ptr.into()];
                for v in interp_vals {
                    call_args.push(v.into());
                }
                self.builder.build_call(printf, &call_args, "print")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
            }

            other => {
                if let Some(val) = self.compile_expr(other)? {
                    // Boolean i1 → "true"/"false"
                    if let BasicValueEnum::IntValue(iv) = val {
                        if iv.get_type().get_bit_width() == 1 {
                            let ts = self.build_global_string("true")?;
                            let fs = self.build_global_string("false")?;
                            let sel = self.builder.build_select(iv, ts, fs, "bool_str")
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                            let fmt_ptr = self.build_global_string("%s")?;
                            self.builder.build_call(printf, &[fmt_ptr.into(), sel.into()], "print")
                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                            return Ok(());
                        }
                    }
                    let (fmt_s, promoted) = match val {
                        BasicValueEnum::IntValue(iv) => {
                            let spec = if iv.get_type().get_bit_width() == 64 { "%lld" } else { "%d" };
                            (spec, val)
                        }
                        BasicValueEnum::FloatValue(f) => {
                            let f64ty = self.ctx.f64_type();
                            let prom: BasicValueEnum = if f.get_type().get_bit_width() < 64 {
                                self.builder.build_float_ext(f, f64ty, "fpext")
                                    .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                            } else { val };
                            ("%g", prom)
                        }
                        BasicValueEnum::PointerValue(_) => ("%s", val),
                        _ => ("%d", val),
                    };
                    let fmt_ptr = self.build_global_string(fmt_s)?;
                    self.builder.build_call(printf, &[fmt_ptr.into(), promoted.into()], "print")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                }
            }
        }
        Ok(())
    }

    fn compile_static_call(
        &mut self,
        class  : &str,
        method : &str,
        args   : &[Expr],
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        let fn_name = format!("{}_{}", class, method);

        if let Some(fn_val) = self.fns.get(&fn_name).copied()
            .or_else(|| self.module.get_function(&fn_name))
        {
            let expected = fn_val.count_params() as usize;
            let mut compiled_args: Vec<BasicValueEnum<'ctx>> = args.iter()
                .filter_map(|a| self.compile_expr(a).ok().flatten())
                .collect();
            // Apply default parameters if fewer args were provided
            if compiled_args.len() < expected {
                let defaults = self.default_params.get(&fn_name).cloned();
                if let Some(defs) = defaults {
                    let start = compiled_args.len();
                    for i in start..expected.min(defs.len()) {
                        if let Some(Some(def_expr)) = defs.get(i) {
                            let def_expr_clone = def_expr.clone();
                            if let Some(v) = self.compile_expr(&def_expr_clone)? {
                                compiled_args.push(v);
                            }
                        }
                    }
                }
            }
            let meta_args: Vec<inkwell::values::BasicMetadataValueEnum> =
                compiled_args.iter().map(|v| (*v).into()).collect();
            let call = self.builder.build_call(fn_val, &meta_args, "call")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            return Ok(call.try_as_basic_value().basic());
        }

        let obj_expr = Expr::Ident(class.to_string());

        if let Some(col_cls) = self.infer_object_class(&obj_expr) {
            if matches!(col_cls.as_str(), "__List" | "__HashMap" | "__Pair") {
                let col_cls_owned = col_cls.clone();
                return self.compile_collection_method(&obj_expr.clone(), &col_cls_owned, method, args);
            }
        }

        if let Some(class_name) = self.infer_object_class(&obj_expr) {
            let instance_fn = format!("{}_{}", class_name, method);
            if let Some(fn_val) = self.fns.get(&instance_fn).copied()
                .or_else(|| self.module.get_function(&instance_fn))
            {
                let this_ptr = if let Some(slot) = self.lookup_var(class).cloned() {
                    self.builder.build_load(slot.ty, slot.ptr, "this_load")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                } else {
                    return Ok(None);
                };

                let compiled_args: Vec<BasicValueEnum<'ctx>> = args.iter()
                    .filter_map(|a| self.compile_expr(a).ok().flatten())
                    .collect();

                let mut meta_args: Vec<inkwell::values::BasicMetadataValueEnum> =
                    vec![this_ptr.into()];
                for v in &compiled_args { meta_args.push((*v).into()); }

                let call = self.builder.build_call(fn_val, &meta_args, "mcall")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                return Ok(call.try_as_basic_value().basic());
            }
        }

        if Self::is_string_method(method) {
            if let Some(slot) = self.lookup_var(class).cloned() {
                if slot.class_name.is_none() {
                    let str_val = self.builder.build_load(slot.ty, slot.ptr, "str_load")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    if let BasicValueEnum::PointerValue(str_ptr) = str_val {
                        let args_cloned = args.to_vec();
                        return self.compile_string_method(str_ptr.into(), method, &args_cloned);
                    }
                }
            }
        }

        Ok(None)
    }

    fn compile_method_call(
        &mut self,
        _object : &Expr,
        _method : &str,
        args    : &[Expr],
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        for a in args { self.compile_expr(a)?; }
        Ok(None)
    }

    fn compile_instance_method_call(
        &mut self,
        object : &Expr,
        method : &str,
        args   : &[Expr],
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        let class_name = self.infer_object_class(object);

        let this_ptr = match self.compile_expr(object)? {
            Some(v) => v,
            None    => return Ok(None),
        };

        // When class_name is None (object is result of expression, not a simple Ident),
        // try string method dispatch if the compiled object is a pointer
        if class_name.is_none() {
            if let BasicValueEnum::PointerValue(_) = this_ptr {
                if Self::is_string_method(method) {
                    return self.compile_string_method(this_ptr, method, args);
                }
            }
            return Ok(None);
        }

        let fn_name = format!("{}_{}", class_name.as_ref().unwrap(), method);

        let fn_val = match self.fns.get(&fn_name).copied()
            .or_else(|| self.module.get_function(&fn_name))
        {
            Some(f) => f,
            None => {
                // Fallback 1: field access (e.g. this.buf where buf is a String field)
                if args.is_empty() {
                    if let (Some(cn), BasicValueEnum::PointerValue(ptr)) =
                        (class_name.as_deref(), this_ptr)
                    {
                        if let Some(loaded) = self.gep_field_load(cn, ptr, method)? {
                            return Ok(Some(loaded));
                        }
                    }
                }
                // Fallback 2: string method on ptr (field that happens to be String)
                if let BasicValueEnum::PointerValue(_) = this_ptr {
                    if Self::is_string_method(method) {
                        return self.compile_string_method(this_ptr, method, args);
                    }
                }
                return Ok(None);
            }
        };

        let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum> = vec![this_ptr.into()];
        for a in args.iter() {
            if let Some(v) = self.compile_expr(a)? {
                call_args.push(v.into());
            }
        }

        let call = self.builder.build_call(fn_val, &call_args, "mcall")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        Ok(call.try_as_basic_value().basic())
    }

    fn compile_field_load(
        &mut self,
        object : &Expr,
        field  : &str,
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        let class_name = self.infer_object_class(object);
        let this_ptr = match self.compile_expr(object)? {
            Some(v) => v,
            None    => return Ok(None),
        };

        if let (Some(cn), BasicValueEnum::PointerValue(ptr)) = (class_name, this_ptr) {
            return self.gep_field_load(&cn, ptr, field);
        }
        Ok(None)
    }

    fn gep_field_load(
        &mut self,
        class : &str,
        ptr   : inkwell::values::PointerValue<'ctx>,
        field : &str,
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        let idx = match self.field_indices.get(class).and_then(|m| m.get(field)).copied() {
            Some(i) => i,
            None    => return Ok(None),
        };
        let struct_ty = match self.struct_types.get(class).copied() {
            Some(t) => t,
            None    => return Ok(None),
        };
        let i32_ty = self.ctx.i32_type();
        let gep = self.builder.build_struct_gep(struct_ty, ptr, idx, field)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let field_ty = struct_ty.get_field_type_at_index(idx)
            .ok_or_else(|| CodeGenError::new(format!("field {} not found", field)))?;
        let val = self.builder.build_load(field_ty, gep, field)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let _ = i32_ty;
        Ok(Some(val))
    }

    fn gep_field_store(
        &mut self,
        class : &str,
        ptr   : inkwell::values::PointerValue<'ctx>,
        field : &str,
        val   : BasicValueEnum<'ctx>,
    ) -> CgResult<()> {
        let idx = match self.field_indices.get(class).and_then(|m| m.get(field)).copied() {
            Some(i) => i,
            None    => return Ok(()),
        };
        let struct_ty = match self.struct_types.get(class).copied() {
            Some(t) => t,
            None    => return Ok(()),
        };
        let gep = self.builder.build_struct_gep(struct_ty, ptr, idx, &format!("{}.ptr", field))
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let field_ty = struct_ty.get_field_type_at_index(idx)
            .ok_or_else(|| CodeGenError::new(format!("field {} not found", field)))?;

        if matches!(field_ty, BasicTypeEnum::PointerType(_)) {
            let field_class = self.field_arimo_types
                .get(class)
                .and_then(|m| m.get(field))
                .cloned()
                .filter(|cn| self.struct_types.contains_key(cn.as_str()));

            if let Some(ref fcn) = field_class {
                if !self.manual_memory_classes.contains(fcn.as_str()) {
                    if let Ok(old_val) = self.builder.build_load(field_ty, gep, "field_old") {
                        if let BasicValueEnum::PointerValue(old_ptr) = old_val {
                            let fcn_clone = fcn.clone();
                            let i64_ty  = self.ctx.i64_type();
                            let ptr_ty  = self.ctx.ptr_type(inkwell::AddressSpace::default());
                            let cur_fn  = self.cur_fn;
                            if let Some(cur_fn) = cur_fn {
                                if !self.current_block_terminated() {
                                    let dec_bb  = self.ctx.append_basic_block(cur_fn, "fs.dec");
                                    let free_bb = self.ctx.append_basic_block(cur_fn, "fs.free");
                                    let cont_bb = self.ctx.append_basic_block(cur_fn, "fs.cont");
                                    let pi = self.builder.build_ptr_to_int(old_ptr, i64_ty, "fs_pi")
                                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                                    let is_null = self.builder.build_int_compare(
                                        inkwell::IntPredicate::EQ, pi, i64_ty.const_int(0, false), "fs_null"
                                    ).map_err(|e| CodeGenError::new(e.to_string()))?;
                                    self.builder.build_conditional_branch(is_null, cont_bb, dec_bb)
                                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                                    self.builder.position_at_end(dec_bb);
                                    if let Some(&rc_idx) = self.refcount_indices.get(&fcn_clone) {
                                        if let Some(sty) = self.struct_types.get(&fcn_clone).copied() {
                                            let rc_gep = self.builder.build_struct_gep(sty, old_ptr, rc_idx, "fs_rc_gep")
                                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                                            let rc_old = self.builder.build_load(i64_ty, rc_gep, "fs_rc")
                                                .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
                                            let rc_new = self.builder.build_int_sub(rc_old, i64_ty.const_int(1, false), "fs_rcdec")
                                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                                            self.builder.build_store(rc_gep, rc_new)
                                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                                            let is_zero = self.builder.build_int_compare(
                                                inkwell::IntPredicate::EQ, rc_new, i64_ty.const_int(0, false), "fs_zero"
                                            ).map_err(|e| CodeGenError::new(e.to_string()))?;
                                            self.builder.build_conditional_branch(is_zero, free_bb, cont_bb)
                                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                                        } else {
                                            self.builder.build_unconditional_branch(cont_bb)
                                                .map_err(|e| CodeGenError::new(e.to_string()))?;
                                        }
                                    } else {
                                        self.builder.build_unconditional_branch(cont_bb)
                                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                                    }
                                    self.builder.position_at_end(free_bb);
                                    let free_fn = self.module.get_function("free").unwrap_or_else(|| {
                                        let ft = self.ctx.void_type().fn_type(&[ptr_ty.into()], false);
                                        self.module.add_function("free", ft, None)
                                    });
                                    self.builder.build_call(free_fn, &[old_ptr.into()], "")
                                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                                    self.builder.build_unconditional_branch(cont_bb)
                                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                                    self.builder.position_at_end(cont_bb);
                                }
                            }
                        }
                    }
                    if let BasicValueEnum::PointerValue(new_ptr) = val {
                        let fcn_clone = fcn.clone();
                        self.arc_retain_ptr(new_ptr, &fcn_clone)?;
                    }
                }
            }
        }

        let coerced = self.coerce_value(val, field_ty)?;
        self.builder.build_store(gep, coerced)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        Ok(())
    }

    fn infer_object_class(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::This => self.cur_class.clone(),
            Expr::Ident(name) => {
                for scope in self.scopes.iter().rev() {
                    if let Some(slot) = scope.get(name.as_str()) {
                        if slot.class_name.is_some() {
                            return slot.class_name.clone();
                        }
                    }
                }
                self.param_class_map.get(name.as_str()).cloned()
            }
            Expr::FieldAccess { object, field } => {
                let owner_class = self.infer_object_class(object)?;
                self.field_arimo_types
                    .get(&owner_class)?
                    .get(field.as_str())
                    .cloned()
            }
            Expr::MethodCall { object, method, .. } => {
                let owner_class = self.infer_object_class(object)?;
                if matches!(owner_class.as_str(), "__List" | "__HashMap" | "__Pair") {
                    if method == "get" {
                        return self.infer_list_elem_class(object);
                    }
                    return None;
                }
                let fn_name = format!("{}_{}", owner_class, method);
                self.fn_return_class.get(&fn_name).cloned()
            }
            Expr::StaticCall { class, method, .. } => {
                let obj_expr = Expr::Ident(class.to_string());
                let owner_class = self.infer_object_class(&obj_expr)?;
                if matches!(owner_class.as_str(), "__List" | "__HashMap" | "__Pair") {
                    if method == "get" {
                        for scope in self.scopes.iter().rev() {
                            if let Some(slot) = scope.get(class.as_str()) {
                                if let Some(ec) = &slot.elem_class {
                                    if self.struct_types.contains_key(ec.as_str()) {
                                        return Some(ec.clone());
                                    }
                                }
                            }
                        }
                        if let Some(ec) = self.param_elem_map.get(class.as_str()) {
                            if self.struct_types.contains_key(ec.as_str()) {
                                return Some(ec.clone());
                            }
                        }
                    }
                    return None;
                }
                let fn_name = format!("{}_{}", owner_class, method);
                self.fn_return_class.get(&fn_name).cloned()
            }
            Expr::ConstructorCall { class, .. } => Some(class.clone()),
            _ => None,
        }
    }

    fn infer_list_elem_class(&self, list_expr: &Expr) -> Option<String> {
        match list_expr {
            Expr::Ident(n) => {
                for scope in self.scopes.iter().rev() {
                    if let Some(slot) = scope.get(n.as_str()) {
                        if let Some(ec) = &slot.elem_class {
                            if self.struct_types.contains_key(ec.as_str()) {
                                return Some(ec.clone());
                            }
                        }
                    }
                }
                self.param_elem_map.get(n.as_str()).cloned()
            }
            Expr::FieldAccess { object, field } => {
                let owner = self.infer_object_class(object)?;
                self.field_elem_classes.get(&owner)?.get(field.as_str()).cloned()
            }
            _ => None,
        }
    }

    fn compile_binop(
        &mut self,
        op    : &BinOp,
        left  : &Expr,
        right : &Expr,
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        if matches!(op, BinOp::Assign) {
            return self.compile_assign(left, right);
        }
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
                (BasicValueEnum::PointerValue(l), BasicValueEnum::PointerValue(r)) => {
                    self.declare_string_fns();
                    let i64_ty = self.ctx.i64_type();
                    let strlen = self.module.get_function("strlen").unwrap();
                    let malloc = self.module.get_function("malloc").unwrap();
                    let strcpy = self.module.get_function("strcpy").unwrap();
                    let strcat = self.module.get_function("strcat").unwrap();
                    let llen_r = self.builder.build_call(strlen, &[l.into()], "llen")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let rlen_r = self.builder.build_call(strlen, &[r.into()], "rlen")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let llen = match llen_r.try_as_basic_value().basic() {
                        Some(BasicValueEnum::IntValue(v)) => v,
                        _ => i64_ty.const_int(0, false),
                    };
                    let rlen = match rlen_r.try_as_basic_value().basic() {
                        Some(BasicValueEnum::IntValue(v)) => v,
                        _ => i64_ty.const_int(0, false),
                    };
                    let total = self.builder.build_int_add(llen, rlen, "concat_len")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let total1 = self.builder.build_int_add(total, i64_ty.const_int(1, false), "concat_len1")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let buf_call = self.builder.build_call(malloc, &[total1.into()], "concat_buf")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let buf_ptr = match buf_call.try_as_basic_value().basic() {
                        Some(BasicValueEnum::PointerValue(p)) => p,
                        _ => return Ok(None),
                    };
                    self.builder.build_call(strcpy, &[buf_ptr.into(), l.into()], "")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_call(strcat, &[buf_ptr.into(), r.into()], "")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    buf_ptr.into()
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
                    let r_cast = self.match_int_width(r, l.get_type())?;
                    self.builder.build_left_shift(l, r_cast, "shl")
                        .map_err(|e| CodeGenError::new(e.to_string()))?.into()
                }
                _ => return Ok(None),
            },
            BinOp::Shr => match (lv, rv) {
                (BasicValueEnum::IntValue(l), BasicValueEnum::IntValue(r)) => {
                    let r_cast = self.match_int_width(r, l.get_type())?;
                    self.builder.build_right_shift(l, r_cast, false, "shr")
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

                        if let Some(ref cn) = slot.class_name.clone() {
                            if !matches!(cn.as_str(), "__List" | "__HashMap" | "__Pair") {
                                self.arc_release_var(VarSlot {
                                    ptr: slot.ptr,
                                    ty: slot.ty,
                                    class_name: Some(cn.clone()),
                                    elem_class: None,
                                    enum_name: None,
                                })?;
                                if let BasicValueEnum::PointerValue(new_ptr) = coerced {
                                    let cn_clone = cn.clone();
                                    self.arc_retain_ptr(new_ptr, &cn_clone)?;
                                }
                            }
                        }

                        self.builder.build_store(slot.ptr, coerced)
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                    }
                }
                Expr::FieldAccess { object, field } => {
                    if let Expr::Ident(class_name) = object.as_ref() {
                        let global_name = format!("{}_{}", class_name, field);
                        if let Some(gv) = self.module.get_global(&global_name) {
                            let ptr = gv.as_pointer_value();
                            if let Some(g_ty) = self.any_to_basic(gv.get_value_type()) {
                                let coerced = self.coerce_value(v, g_ty)?;
                                self.builder.build_store(ptr, coerced)
                                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                            }
                        } else {
                            let cn = self.infer_object_class(object);
                            let obj_ptr = self.compile_expr(object)?;
                            if let (Some(cn), Some(BasicValueEnum::PointerValue(ptr))) = (cn, obj_ptr) {
                                self.gep_field_store(&cn.clone(), ptr, field, v)?;
                            }
                        }
                    } else {
                        let class_name = self.infer_object_class(object);
                        let obj_ptr = self.compile_expr(object)?;
                        if let (Some(cn), Some(BasicValueEnum::PointerValue(ptr))) = (class_name, obj_ptr) {
                            self.gep_field_store(&cn.clone(), ptr, field, v)?;
                        }
                    }
                }
                // this.field = val where field access is represented as zero-arg MethodCall
                Expr::MethodCall { object, method, args } if args.is_empty() => {
                    let cn = self.infer_object_class(object);
                    let obj_ptr = self.compile_expr(object)?;
                    if let (Some(cn), Some(BasicValueEnum::PointerValue(ptr))) = (cn, obj_ptr) {
                        self.gep_field_store(&cn.clone(), ptr, method, v)?;
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

    fn build_global_string(&mut self, s: &str) -> CgResult<PointerValue<'ctx>> {
        let gs = self.builder.build_global_string_ptr(s, "str")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        Ok(gs.as_pointer_value())
    }

    fn any_to_basic(&self, any: inkwell::types::AnyTypeEnum<'ctx>) -> Option<BasicTypeEnum<'ctx>> {
        use inkwell::types::AnyTypeEnum;
        match any {
            AnyTypeEnum::IntType(t)     => Some(t.into()),
            AnyTypeEnum::FloatType(t)   => Some(t.into()),
            AnyTypeEnum::PointerType(t) => Some(t.into()),
            AnyTypeEnum::ArrayType(t)   => Some(t.into()),
            AnyTypeEnum::StructType(t)  => Some(t.into()),
            AnyTypeEnum::VectorType(t)  => Some(t.into()),
            _ => None,
        }
    }

    fn match_int_width(
        &mut self,
        val  : inkwell::values::IntValue<'ctx>,
        dest : inkwell::types::IntType<'ctx>,
    ) -> CgResult<inkwell::values::IntValue<'ctx>> {
        let src_bits = val.get_type().get_bit_width();
        let dst_bits = dest.get_bit_width();
        if src_bits == dst_bits { return Ok(val); }
        if src_bits < dst_bits {
            self.builder.build_int_z_extend(val, dest, "shamt_zext")
                .map_err(|e| CodeGenError::new(e.to_string()))
        } else {
            self.builder.build_int_truncate(val, dest, "shamt_trunc")
                .map_err(|e| CodeGenError::new(e.to_string()))
        }
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
        let i64_ty = self.ctx.i64_type();
        match (l, r) {
            (BasicValueEnum::IntValue(a), BasicValueEnum::IntValue(b)) =>
                self.builder.build_int_compare(inkwell::IntPredicate::EQ, a, b, "eq")
                    .map_err(|e| CodeGenError::new(e.to_string())),
            (BasicValueEnum::FloatValue(a), BasicValueEnum::FloatValue(b)) =>
                self.builder.build_float_compare(inkwell::FloatPredicate::OEQ, a, b, "feq")
                    .map_err(|e| CodeGenError::new(e.to_string())),
            (BasicValueEnum::PointerValue(a), BasicValueEnum::PointerValue(b)) => {
                // null check: pointer comparison; string comparison: strcmp
                if a.is_null() || b.is_null() {
                    let ai = self.builder.build_ptr_to_int(a, i64_ty, "eq_pi_a")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let bi = self.builder.build_ptr_to_int(b, i64_ty, "eq_pi_b")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_int_compare(inkwell::IntPredicate::EQ, ai, bi, "ptr_eq")
                        .map_err(|e| CodeGenError::new(e.to_string()))
                } else {
                    self.declare_strcmp();
                    let strcmp = self.module.get_function("strcmp").unwrap();
                    let i32_ty = self.ctx.i32_type();
                    let cmp = self.builder.build_call(strcmp, &[a.into(), b.into()], "streq_cmp")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let cmp_i = match cmp.try_as_basic_value().basic() {
                        Some(BasicValueEnum::IntValue(iv)) => iv,
                        _ => i32_ty.const_int(0, false),
                    };
                    self.builder.build_int_compare(
                        inkwell::IntPredicate::EQ, cmp_i, i32_ty.const_int(0, false), "str_eq")
                        .map_err(|e| CodeGenError::new(e.to_string()))
                }
            }
            (BasicValueEnum::IntValue(a), BasicValueEnum::PointerValue(b)) => {
                let bi = self.builder.build_ptr_to_int(b, i64_ty, "eq_pi_b")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let a64 = if a.get_type().get_bit_width() < 64 {
                    self.builder.build_int_z_extend(a, i64_ty, "zext_a")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                } else { a };
                self.builder.build_int_compare(inkwell::IntPredicate::EQ, a64, bi, "pi_eq")
                    .map_err(|e| CodeGenError::new(e.to_string()))
            }
            _ => Ok(self.ctx.bool_type().const_int(0, false)),
        }
    }

    fn build_ne(&mut self, l: BasicValueEnum<'ctx>, r: BasicValueEnum<'ctx>)
        -> CgResult<inkwell::values::IntValue<'ctx>>
    {
        let i64_ty = self.ctx.i64_type();
        match (l, r) {
            (BasicValueEnum::IntValue(a), BasicValueEnum::IntValue(b)) =>
                self.builder.build_int_compare(inkwell::IntPredicate::NE, a, b, "ne")
                    .map_err(|e| CodeGenError::new(e.to_string())),
            (BasicValueEnum::FloatValue(a), BasicValueEnum::FloatValue(b)) =>
                self.builder.build_float_compare(inkwell::FloatPredicate::ONE, a, b, "fne")
                    .map_err(|e| CodeGenError::new(e.to_string())),
            (BasicValueEnum::PointerValue(a), BasicValueEnum::PointerValue(b)) => {
                if a.is_null() || b.is_null() {
                    let ai = self.builder.build_ptr_to_int(a, i64_ty, "ne_pi_a")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let bi = self.builder.build_ptr_to_int(b, i64_ty, "ne_pi_b")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_int_compare(inkwell::IntPredicate::NE, ai, bi, "ptr_ne")
                        .map_err(|e| CodeGenError::new(e.to_string()))
                } else {
                    self.declare_strcmp();
                    let strcmp = self.module.get_function("strcmp").unwrap();
                    let i32_ty = self.ctx.i32_type();
                    let cmp = self.builder.build_call(strcmp, &[a.into(), b.into()], "strne_cmp")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let cmp_i = match cmp.try_as_basic_value().basic() {
                        Some(BasicValueEnum::IntValue(iv)) => iv,
                        _ => i32_ty.const_int(0, false),
                    };
                    self.builder.build_int_compare(
                        inkwell::IntPredicate::NE, cmp_i, i32_ty.const_int(0, false), "str_ne")
                        .map_err(|e| CodeGenError::new(e.to_string()))
                }
            }
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

    fn build_list_map(
        &mut self,
        src_ptr : inkwell::values::PointerValue<'ctx>,
        fn_ptr  : BasicValueEnum<'ctx>,
    ) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        let i64_ty = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let is_fat = matches!(fn_ptr, BasicValueEnum::StructValue(_));
        let (fn_p, cl_ptr) = self.extract_fn_closure(fn_ptr)?;
        let fn_type = if is_fat {
            i64_ty.fn_type(&[i64_ty.into(), ptr_ty.into()], false)
        } else {
            i64_ty.fn_type(&[i64_ty.into()], false)
        };

        let list_len_fn    = self.module.get_function("arc_list_length").unwrap();
        let list_get_fn    = self.module.get_function("arc_list_get").unwrap();
        let list_new_fn    = self.module.get_function("arc_list_new").unwrap();
        let list_append_fn = self.module.get_function("arc_list_append").unwrap();

        let result_call = self.builder.build_call(list_new_fn, &[], "map_result")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let result_ptr = match result_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(ptr_ty.const_null()),
        };
        let len_call = self.builder.build_call(list_len_fn, &[src_ptr.into()], "map_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let len = match len_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => return Ok(result_ptr),
        };

        let cur_fn = self.cur_fn.unwrap();
        let idx_alloca = self.builder.build_alloca(i64_ty, "map_idx")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let cond_bb = self.ctx.append_basic_block(cur_fn, "map.cond");
        let body_bb = self.ctx.append_basic_block(cur_fn, "map.body");
        let exit_bb = self.ctx.append_basic_block(cur_fn, "map.exit");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(cond_bb);
        let idx = self.builder.build_load(i64_ty, idx_alloca, "map_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let cond = self.builder.build_int_compare(inkwell::IntPredicate::SLT, idx, len, "map_cond")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(cond, body_bb, exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(body_bb);
        let item_call = self.builder.build_call(list_get_fn, &[src_ptr.into(), idx.into()], "map_item")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let item = item_call.try_as_basic_value().basic()
            .unwrap_or(i64_ty.const_int(0, false).into());
        let item_i64 = self.value_to_i64(item)?;

        let map_call_args: Vec<inkwell::values::BasicMetadataValueEnum> = if is_fat {
            vec![item_i64.into(), cl_ptr.into()]
        } else {
            vec![item_i64.into()]
        };
        let mapped = self.builder.build_indirect_call(fn_type, fn_p, &map_call_args, "map_call")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        if let Some(mapped_val) = mapped.try_as_basic_value().basic() {
            let mapped_i64 = self.value_to_i64(mapped_val)?;
            self.builder.build_call(list_append_fn, &[result_ptr.into(), mapped_i64.into()], "")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }

        let next = self.builder.build_int_add(idx, i64_ty.const_int(1, false), "map_inc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(exit_bb);
        Ok(result_ptr)
    }

    fn build_list_any_all(
        &mut self,
        src_ptr : inkwell::values::PointerValue<'ctx>,
        fn_ptr  : BasicValueEnum<'ctx>,
        is_any  : bool,
    ) -> CgResult<inkwell::values::IntValue<'ctx>> {
        let i64_ty = self.ctx.i64_type();
        let i1_ty  = self.ctx.bool_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let is_fat_aa = matches!(fn_ptr, BasicValueEnum::StructValue(_));
        let (fn_p_aa, cl_ptr_aa) = self.extract_fn_closure(fn_ptr)?;
        let fn_type = if is_fat_aa {
            i64_ty.fn_type(&[i64_ty.into(), ptr_ty.into()], false)
        } else {
            i64_ty.fn_type(&[i64_ty.into()], false)
        };
        let list_len_fn = self.module.get_function("arc_list_length").unwrap();
        let list_get_fn = self.module.get_function("arc_list_get").unwrap();

        let len_call = self.builder.build_call(list_len_fn, &[src_ptr.into()], "aa_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let len = match len_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => return Ok(i1_ty.const_int(if is_any { 0 } else { 1 }, false)),
        };

        let cur_fn    = self.cur_fn.unwrap();
        let idx_alloca = self.builder.build_alloca(i64_ty, "aa_idx")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let res_alloca = self.builder.build_alloca(i1_ty, "aa_res")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(res_alloca, i1_ty.const_int(if is_any { 0 } else { 1 }, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let cond_bb  = self.ctx.append_basic_block(cur_fn, "aa.cond");
        let body_bb  = self.ctx.append_basic_block(cur_fn, "aa.body");
        let early_bb = self.ctx.append_basic_block(cur_fn, "aa.early");
        let exit_bb  = self.ctx.append_basic_block(cur_fn, "aa.exit");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(cond_bb);
        let idx = self.builder.build_load(i64_ty, idx_alloca, "aa_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let cond = self.builder.build_int_compare(inkwell::IntPredicate::SLT, idx, len, "aa_cond")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(cond, body_bb, exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(body_bb);
        let item_call = self.builder.build_call(list_get_fn, &[src_ptr.into(), idx.into()], "aa_item")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let item = item_call.try_as_basic_value().basic()
            .unwrap_or(i64_ty.const_int(0, false).into());
        let item_i64 = self.value_to_i64(item)?;

        let aa_call_args: Vec<inkwell::values::BasicMetadataValueEnum> = if is_fat_aa {
            vec![item_i64.into(), cl_ptr_aa.into()]
        } else {
            vec![item_i64.into()]
        };
        let pred_result = {
            let r = self.builder.build_indirect_call(fn_type, fn_p_aa, &aa_call_args, "aa_pred")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            r.try_as_basic_value().basic()
                .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) } else { None })
        };

        if let Some(pred_i64) = pred_result {
            let is_nonzero = self.builder.build_int_compare(
                inkwell::IntPredicate::NE, pred_i64, i64_ty.const_int(0, false), "aa_nz"
            ).map_err(|e| CodeGenError::new(e.to_string()))?;
            let trigger = if is_any { is_nonzero } else {
                self.builder.build_not(is_nonzero, "aa_not")
                    .map_err(|e| CodeGenError::new(e.to_string()))?
            };
            self.builder.build_conditional_branch(trigger, early_bb, cond_bb)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        } else {
            self.builder.build_unconditional_branch(cond_bb)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }

        let next = self.builder.build_int_add(idx, i64_ty.const_int(1, false), "aa_inc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(early_bb);
        self.builder.build_store(res_alloca, i1_ty.const_int(if is_any { 1 } else { 0 }, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(exit_bb);
        let res = self.builder.build_load(i1_ty, res_alloca, "aa_result")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        Ok(res)
    }

    fn build_list_reduce(
        &mut self,
        src_ptr  : inkwell::values::PointerValue<'ctx>,
        init     : inkwell::values::IntValue<'ctx>,
        fn_ptr   : BasicValueEnum<'ctx>,
    ) -> CgResult<inkwell::values::IntValue<'ctx>> {
        let i64_ty  = self.ctx.i64_type();
        let ptr_ty  = self.ctx.ptr_type(AddressSpace::default());
        let is_fat_red = matches!(fn_ptr, BasicValueEnum::StructValue(_));
        let (fn_p_red, cl_ptr_red) = self.extract_fn_closure(fn_ptr)?;
        let fn_type = if is_fat_red {
            i64_ty.fn_type(&[i64_ty.into(), i64_ty.into(), ptr_ty.into()], false)
        } else {
            i64_ty.fn_type(&[i64_ty.into(), i64_ty.into()], false)
        };
        let list_len_fn = self.module.get_function("arc_list_length").unwrap();
        let list_get_fn = self.module.get_function("arc_list_get").unwrap();

        let len_call = self.builder.build_call(list_len_fn, &[src_ptr.into()], "red_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let len = match len_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => return Ok(init),
        };

        let cur_fn    = self.cur_fn.unwrap();
        let acc_alloca = self.builder.build_alloca(i64_ty, "red_acc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(acc_alloca, init)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let idx_alloca = self.builder.build_alloca(i64_ty, "red_idx")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let cond_bb = self.ctx.append_basic_block(cur_fn, "red.cond");
        let body_bb = self.ctx.append_basic_block(cur_fn, "red.body");
        let exit_bb = self.ctx.append_basic_block(cur_fn, "red.exit");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(cond_bb);
        let idx = self.builder.build_load(i64_ty, idx_alloca, "red_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let cond = self.builder.build_int_compare(inkwell::IntPredicate::SLT, idx, len, "red_c")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(cond, body_bb, exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(body_bb);
        let acc = self.builder.build_load(i64_ty, acc_alloca, "red_acc_v")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let item_call = self.builder.build_call(list_get_fn, &[src_ptr.into(), idx.into()], "red_item")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let item = item_call.try_as_basic_value().basic()
            .unwrap_or(i64_ty.const_int(0, false).into());
        let item_i64 = self.value_to_i64(item)?;

        let red_call_args: Vec<inkwell::values::BasicMetadataValueEnum> = if is_fat_red {
            vec![acc.into(), item_i64.into(), cl_ptr_red.into()]
        } else {
            vec![acc.into(), item_i64.into()]
        };
        let new_acc = self.builder.build_indirect_call(fn_type, fn_p_red, &red_call_args, "red_call")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        if let Some(BasicValueEnum::IntValue(na)) = new_acc.try_as_basic_value().basic() {
            self.builder.build_store(acc_alloca, na)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }

        let next = self.builder.build_int_add(idx, i64_ty.const_int(1, false), "red_inc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(exit_bb);
        let result = self.builder.build_load(i64_ty, acc_alloca, "red_result")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        Ok(result)
    }

    fn build_list_take(
        &mut self,
        src_ptr   : inkwell::values::PointerValue<'ctx>,
        n         : inkwell::values::IntValue<'ctx>,
        from_last : bool,
    ) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        let i64_ty = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let list_len_fn    = self.module.get_function("arc_list_length").unwrap();
        let list_get_fn    = self.module.get_function("arc_list_get").unwrap();
        let list_new_fn    = self.module.get_function("arc_list_new").unwrap();
        let list_append_fn = self.module.get_function("arc_list_append").unwrap();

        let result_call = self.builder.build_call(list_new_fn, &[], "take_result")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let result_ptr = match result_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(ptr_ty.const_null()),
        };

        let len_call = self.builder.build_call(list_len_fn, &[src_ptr.into()], "take_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let len = match len_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => return Ok(result_ptr),
        };

        let start = if from_last {
            let diff = self.builder.build_int_sub(len, n, "take_diff")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            let zero = i64_ty.const_int(0, false);
            let is_neg = self.builder.build_int_compare(inkwell::IntPredicate::SLT, diff, zero, "take_neg")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            self.builder.build_select(is_neg, zero, diff, "take_start")
                .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value()
        } else {
            i64_ty.const_int(0, false)
        };

        let end = if from_last {
            len
        } else {
            let is_over = self.builder.build_int_compare(inkwell::IntPredicate::SGT, n, len, "take_over")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            self.builder.build_select(is_over, len, n, "take_end")
                .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value()
        };

        let cur_fn = self.cur_fn.unwrap();
        let idx_alloca = self.builder.build_alloca(i64_ty, "take_idx")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, start)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let cond_bb = self.ctx.append_basic_block(cur_fn, "take.cond");
        let body_bb = self.ctx.append_basic_block(cur_fn, "take.body");
        let exit_bb = self.ctx.append_basic_block(cur_fn, "take.exit");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(cond_bb);
        let idx = self.builder.build_load(i64_ty, idx_alloca, "take_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let cond = self.builder.build_int_compare(inkwell::IntPredicate::SLT, idx, end, "take_cond")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(cond, body_bb, exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(body_bb);
        let item_call = self.builder.build_call(list_get_fn, &[src_ptr.into(), idx.into()], "take_item")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        if let Some(item) = item_call.try_as_basic_value().basic() {
            let item_i64 = self.value_to_i64(item)?;
            self.builder.build_call(list_append_fn, &[result_ptr.into(), item_i64.into()], "")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }
        let next = self.builder.build_int_add(idx, i64_ty.const_int(1, false), "take_inc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(exit_bb);
        Ok(result_ptr)
    }

    fn build_list_sorted_by(
        &mut self,
        src_ptr : inkwell::values::PointerValue<'ctx>,
        fn_ptr  : BasicValueEnum<'ctx>,
    ) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        let i64_ty     = self.ctx.i64_type();
        let ptr_ty     = self.ctx.ptr_type(AddressSpace::default());
        let list_new   = self.module.get_function("arc_list_new").unwrap();
        let list_app   = self.module.get_function("arc_list_append").unwrap();
        let list_get   = self.module.get_function("arc_list_length").unwrap();
        let list_getf  = self.module.get_function("arc_list_get").unwrap();
        let list_set   = self.module.get_function("arc_list_set").unwrap();

        let is_fat = matches!(fn_ptr, BasicValueEnum::StructValue(_));
        let (fn_p, cl_ptr) = self.extract_fn_closure(fn_ptr)?;
        let fn_type = if is_fat {
            i64_ty.fn_type(&[i64_ty.into(), i64_ty.into(), ptr_ty.into()], false)
        } else {
            i64_ty.fn_type(&[i64_ty.into(), i64_ty.into()], false)
        };

        let res_call = self.builder.build_call(list_new, &[], "srt_res")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let result_ptr = match res_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(ptr_ty.const_null()),
        };
        let len_call = self.builder.build_call(list_get, &[src_ptr.into()], "srt_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let len = match len_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => return Ok(result_ptr),
        };

        let cur_fn = self.cur_fn.unwrap();

        let cp_idx      = self.builder.build_alloca(i64_ty, "srt_ci")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let pass_alloca = self.builder.build_alloca(i64_ty, "srt_pass")
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.build_store(cp_idx, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let cp_cond = self.ctx.append_basic_block(cur_fn, "srt.cp.cond");
        let cp_body = self.ctx.append_basic_block(cur_fn, "srt.cp.body");
        let sort_bb = self.ctx.append_basic_block(cur_fn, "srt.outer.init");
        self.builder.build_unconditional_branch(cp_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(cp_cond);
        let ci = self.builder.build_load(i64_ty, cp_idx, "srt_ci_v")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let cp_ok = self.builder.build_int_compare(inkwell::IntPredicate::SLT, ci, len, "srt_cp_ok")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(cp_ok, cp_body, sort_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(cp_body);
        let elem = self.builder.build_call(list_getf, &[src_ptr.into(), ci.into()], "srt_e")
            .map_err(|e| CodeGenError::new(e.to_string()))?
            .try_as_basic_value().basic()
            .unwrap_or(i64_ty.const_int(0, false).into());
        let elem_i64 = self.value_to_i64(elem)?;
        self.builder.build_call(list_app, &[result_ptr.into(), elem_i64.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let ci_next = self.builder.build_int_add(ci, i64_ty.const_int(1, false), "srt_ci_n")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(cp_idx, ci_next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(cp_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(sort_bb);
        self.builder.build_store(pass_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let o_cond = self.ctx.append_basic_block(cur_fn, "srt.o.cond");
        let o_body = self.ctx.append_basic_block(cur_fn, "srt.o.body");
        let srt_exit = self.ctx.append_basic_block(cur_fn, "srt.exit");
        self.builder.build_unconditional_branch(o_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(o_cond);
        let pass = self.builder.build_load(i64_ty, pass_alloca, "srt_pass_v")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let o_ok = self.builder.build_int_compare(inkwell::IntPredicate::SLT, pass, len, "srt_o_ok")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(o_ok, o_body, srt_exit)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(o_body);
        let i_alloca = self.builder.build_alloca(i64_ty, "srt_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(i_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let len_m1 = self.builder.build_int_sub(len, i64_ty.const_int(1, false), "srt_lm1")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let i_cond = self.ctx.append_basic_block(cur_fn, "srt.i.cond");
        let i_body = self.ctx.append_basic_block(cur_fn, "srt.i.body");
        let swap_bb = self.ctx.append_basic_block(cur_fn, "srt.swap");
        let no_swap = self.ctx.append_basic_block(cur_fn, "srt.noswap");
        let i_next  = self.ctx.append_basic_block(cur_fn, "srt.i.next");
        let o_next  = self.ctx.append_basic_block(cur_fn, "srt.o.next");
        self.builder.build_unconditional_branch(i_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(i_cond);
        let i_v = self.builder.build_load(i64_ty, i_alloca, "srt_i_v")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let i_ok = self.builder.build_int_compare(inkwell::IntPredicate::SLT, i_v, len_m1, "srt_i_ok")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(i_ok, i_body, o_next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(i_body);
        let iv2 = self.builder.build_load(i64_ty, i_alloca, "srt_iv2")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let i1  = self.builder.build_int_add(iv2, i64_ty.const_int(1, false), "srt_i1")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let a_call = self.builder.build_call(list_getf, &[result_ptr.into(), iv2.into()], "srt_a")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let a = a_call.try_as_basic_value().basic()
            .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) } else { None })
            .unwrap_or(i64_ty.const_int(0, false));
        let b_call = self.builder.build_call(list_getf, &[result_ptr.into(), i1.into()], "srt_b")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let b = b_call.try_as_basic_value().basic()
            .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) } else { None })
            .unwrap_or(i64_ty.const_int(0, false));
        let cmp_args: Vec<inkwell::values::BasicMetadataValueEnum> = if is_fat {
            vec![a.into(), b.into(), cl_ptr.into()]
        } else {
            vec![a.into(), b.into()]
        };
        let cmp_call = self.builder.build_indirect_call(fn_type, fn_p, &cmp_args, "srt_cmp")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let cmp_val = cmp_call.try_as_basic_value().basic()
            .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) } else { None })
            .unwrap_or(i64_ty.const_int(0, false));
        let need_swap = self.builder.build_int_compare(
            inkwell::IntPredicate::SGT, cmp_val, i64_ty.const_int(0, false), "srt_ns")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(need_swap, swap_bb, no_swap)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(swap_bb);
        let iv3 = self.builder.build_load(i64_ty, i_alloca, "srt_iv3")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let i1b = self.builder.build_int_add(iv3, i64_ty.const_int(1, false), "srt_i1b")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_call(list_set, &[result_ptr.into(), iv3.into(), b.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_call(list_set, &[result_ptr.into(), i1b.into(), a.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(i_next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(no_swap);
        self.builder.build_unconditional_branch(i_next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(i_next);
        let iv4 = self.builder.build_load(i64_ty, i_alloca, "srt_iv4")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let in2 = self.builder.build_int_add(iv4, i64_ty.const_int(1, false), "srt_in")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(i_alloca, in2)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(i_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(o_next);
        let pv2 = self.builder.build_load(i64_ty, pass_alloca, "srt_pv2")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let pn  = self.builder.build_int_add(pv2, i64_ty.const_int(1, false), "srt_pn")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(pass_alloca, pn)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(o_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(srt_exit);
        Ok(result_ptr)
    }

    fn build_list_flat_map(
        &mut self,
        src_ptr : inkwell::values::PointerValue<'ctx>,
        fn_ptr  : BasicValueEnum<'ctx>,
    ) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        let i64_ty    = self.ctx.i64_type();
        let ptr_ty    = self.ctx.ptr_type(AddressSpace::default());
        let list_new  = self.module.get_function("arc_list_new").unwrap();
        let list_app  = self.module.get_function("arc_list_append").unwrap();
        let list_len  = self.module.get_function("arc_list_length").unwrap();
        let list_get  = self.module.get_function("arc_list_get").unwrap();

        let is_fat = matches!(fn_ptr, BasicValueEnum::StructValue(_));
        let (fn_p, cl_ptr) = self.extract_fn_closure(fn_ptr)?;
        let fn_type = if is_fat {
            i64_ty.fn_type(&[i64_ty.into(), ptr_ty.into()], false)
        } else {
            i64_ty.fn_type(&[i64_ty.into()], false)
        };

        let res_call = self.builder.build_call(list_new, &[], "fm_res")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let result_ptr = match res_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(ptr_ty.const_null()),
        };
        let src_len_call = self.builder.build_call(list_len, &[src_ptr.into()], "fm_slen")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let src_len = match src_len_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => return Ok(result_ptr),
        };

        let cur_fn = self.cur_fn.unwrap();
        let oi = self.builder.build_alloca(i64_ty, "fm_oi")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(oi, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let o_cond = self.ctx.append_basic_block(cur_fn, "fm.o.cond");
        let o_body = self.ctx.append_basic_block(cur_fn, "fm.o.body");
        let fm_exit= self.ctx.append_basic_block(cur_fn, "fm.exit");
        self.builder.build_unconditional_branch(o_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(o_cond);
        let oiv = self.builder.build_load(i64_ty, oi, "fm_oiv")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let o_ok = self.builder.build_int_compare(inkwell::IntPredicate::SLT, oiv, src_len, "fm_o_ok")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(o_ok, o_body, fm_exit)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(o_body);
        let oiv2 = self.builder.build_load(i64_ty, oi, "fm_oiv2")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let elem_call = self.builder.build_call(list_get, &[src_ptr.into(), oiv2.into()], "fm_elem")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let elem_i64 = elem_call.try_as_basic_value().basic()
            .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) } else { None })
            .unwrap_or(i64_ty.const_int(0, false));

        let lam_args: Vec<inkwell::values::BasicMetadataValueEnum> = if is_fat {
            vec![elem_i64.into(), cl_ptr.into()]
        } else {
            vec![elem_i64.into()]
        };
        let inner_call = self.builder.build_indirect_call(fn_type, fn_p, &lam_args, "fm_inner")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let inner_i64 = match inner_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => {
                let o_next = self.ctx.append_basic_block(cur_fn, "fm.o.next_skip");
                self.builder.build_unconditional_branch(o_next)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.position_at_end(o_next);
                let ov_n = self.builder.build_int_add(oiv2, i64_ty.const_int(1, false), "fm_on")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_store(oi, ov_n)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_unconditional_branch(o_cond)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                return Ok(result_ptr);
            }
        };
        let inner_ptr = self.builder.build_int_to_ptr(inner_i64, ptr_ty, "fm_iptr")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let inner_len_call = self.builder.build_call(list_len, &[inner_ptr.into()], "fm_ilen")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let inner_len = match inner_len_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => i64_ty.const_int(0, false),
        };

        let ji = self.builder.build_alloca(i64_ty, "fm_ji")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(ji, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let i_cond = self.ctx.append_basic_block(cur_fn, "fm.i.cond");
        let i_body = self.ctx.append_basic_block(cur_fn, "fm.i.body");
        let o_next = self.ctx.append_basic_block(cur_fn, "fm.o.next");
        self.builder.build_unconditional_branch(i_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(i_cond);
        let jiv = self.builder.build_load(i64_ty, ji, "fm_jiv")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let j_ok = self.builder.build_int_compare(inkwell::IntPredicate::SLT, jiv, inner_len, "fm_j_ok")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(j_ok, i_body, o_next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(i_body);
        let jiv2 = self.builder.build_load(i64_ty, ji, "fm_jiv2")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let ie_call = self.builder.build_call(list_get, &[inner_ptr.into(), jiv2.into()], "fm_ie")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let ie_val = ie_call.try_as_basic_value().basic()
            .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) } else { None })
            .unwrap_or(i64_ty.const_int(0, false));
        self.builder.build_call(list_app, &[result_ptr.into(), ie_val.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let j_next = self.builder.build_int_add(jiv2, i64_ty.const_int(1, false), "fm_jn")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(ji, j_next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(i_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(o_next);
        let o_n = self.builder.build_int_add(oiv2, i64_ty.const_int(1, false), "fm_on2")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(oi, o_n)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(o_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(fm_exit);
        Ok(result_ptr)
    }

    fn build_map_remove(
        &mut self,
        map_ptr : inkwell::values::PointerValue<'ctx>,
        key_ptr : inkwell::values::PointerValue<'ctx>,
    ) -> CgResult<()> {
        let i64_ty  = self.ctx.i64_type();
        let strcmp  = self.module.get_function("strcmp").unwrap();
        let cur_fn  = self.cur_fn.unwrap();

        let len = self.builder.build_load(i64_ty, map_ptr, "mr_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();

        let idx_a  = self.builder.build_alloca(i64_ty, "mr_idx")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_a, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let cond_bb  = self.ctx.append_basic_block(cur_fn, "mr.cond");
        let chk_bb   = self.ctx.append_basic_block(cur_fn, "mr.chk");
        let found_bb = self.ctx.append_basic_block(cur_fn, "mr.found");
        let next_bb  = self.ctx.append_basic_block(cur_fn, "mr.next");
        let done_bb  = self.ctx.append_basic_block(cur_fn, "mr.done");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(cond_bb);
        let idx = self.builder.build_load(i64_ty, idx_a, "mr_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let lt = self.builder.build_int_compare(inkwell::IntPredicate::SLT, idx, len, "mr_lt")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(lt, chk_bb, done_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(chk_bb);
        let two   = i64_ty.const_int(2, false);
        let one   = i64_ty.const_int(1, false);
        let i2    = self.builder.build_int_mul(idx, two, "mr_i2")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let kslot_idx = self.builder.build_int_add(i2, one, "mr_ks")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let k_gep = unsafe { self.builder.build_gep(i64_ty, map_ptr, &[kslot_idx], "mr_kp")
            .map_err(|e| CodeGenError::new(e.to_string()))? };
        let stored_ki = self.builder.build_load(i64_ty, k_gep, "mr_ski")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let ptr_ty  = self.ctx.ptr_type(AddressSpace::default());
        let stored_kp = self.builder.build_int_to_ptr(stored_ki, ptr_ty, "mr_skp")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let cmp = self.builder.build_call(strcmp, &[stored_kp.into(), key_ptr.into()], "mr_cmp")
            .map_err(|e| CodeGenError::new(e.to_string()))?
            .try_as_basic_value().basic().unwrap().into_int_value();
        let is_eq = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ, cmp, self.ctx.i32_type().const_int(0, false), "mr_eq")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(is_eq, found_bb, next_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(found_bb);
        let idx2 = self.builder.build_load(i64_ty, idx_a, "mr_i2v")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let len2 = self.builder.build_load(i64_ty, map_ptr, "mr_len2")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let last = self.builder.build_int_sub(len2, one, "mr_last")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let lk2  = self.builder.build_int_mul(last, two, "mr_lk2")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let lks  = self.builder.build_int_add(lk2, one, "mr_lks")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let lvs  = self.builder.build_int_add(lk2, two, "mr_lvs")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let lk_gep = unsafe { self.builder.build_gep(i64_ty, map_ptr, &[lks], "mr_lkp")
            .map_err(|e| CodeGenError::new(e.to_string()))? };
        let lv_gep = unsafe { self.builder.build_gep(i64_ty, map_ptr, &[lvs], "mr_lvp")
            .map_err(|e| CodeGenError::new(e.to_string()))? };
        let last_k = self.builder.build_load(i64_ty, lk_gep, "mr_lk_v")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let last_v = self.builder.build_load(i64_ty, lv_gep, "mr_lv_v")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let i2v  = self.builder.build_int_mul(idx2, two, "mr_i2vx")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let tks  = self.builder.build_int_add(i2v, one, "mr_tks")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let tvs  = self.builder.build_int_add(i2v, two, "mr_tvs")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let tk_gep = unsafe { self.builder.build_gep(i64_ty, map_ptr, &[tks], "mr_tkp")
            .map_err(|e| CodeGenError::new(e.to_string()))? };
        let tv_gep = unsafe { self.builder.build_gep(i64_ty, map_ptr, &[tvs], "mr_tvp")
            .map_err(|e| CodeGenError::new(e.to_string()))? };
        self.builder.build_store(tk_gep, last_k)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(tv_gep, last_v)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let new_len = self.builder.build_int_sub(len2, one, "mr_nl")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(map_ptr, new_len)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(done_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(next_bb);
        let idx_n = self.builder.build_int_add(idx, one, "mr_in")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_a, idx_n)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(done_bb);
        Ok(())
    }

    fn build_map_foreach(
        &mut self,
        map_ptr : inkwell::values::PointerValue<'ctx>,
        fn_ptr  : BasicValueEnum<'ctx>,
    ) -> CgResult<()> {
        let i64_ty  = self.ctx.i64_type();
        let ptr_ty  = self.ctx.ptr_type(AddressSpace::default());
        let is_fat  = matches!(fn_ptr, BasicValueEnum::StructValue(_));
        let (fn_p, cl_ptr) = self.extract_fn_closure(fn_ptr)?;
        let fn_type = if is_fat {
            i64_ty.fn_type(&[i64_ty.into(), i64_ty.into(), ptr_ty.into()], false)
        } else {
            i64_ty.fn_type(&[i64_ty.into(), i64_ty.into()], false)
        };

        let len = self.builder.build_load(i64_ty, map_ptr, "mfe_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();

        let cur_fn = self.cur_fn.unwrap();
        let idx_a  = self.builder.build_alloca(i64_ty, "mfe_idx")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_a, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let cond_bb = self.ctx.append_basic_block(cur_fn, "mfe.cond");
        let body_bb = self.ctx.append_basic_block(cur_fn, "mfe.body");
        let exit_bb = self.ctx.append_basic_block(cur_fn, "mfe.exit");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(cond_bb);
        let idx = self.builder.build_load(i64_ty, idx_a, "mfe_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let lt  = self.builder.build_int_compare(inkwell::IntPredicate::SLT, idx, len, "mfe_lt")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(lt, body_bb, exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(body_bb);
        let two  = i64_ty.const_int(2, false);
        let one  = i64_ty.const_int(1, false);
        let i2   = self.builder.build_int_mul(idx, two, "mfe_i2")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let ks   = self.builder.build_int_add(i2, one, "mfe_ks")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let vs   = self.builder.build_int_add(i2, two, "mfe_vs")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let k_gep = unsafe { self.builder.build_gep(i64_ty, map_ptr, &[ks], "mfe_kp")
            .map_err(|e| CodeGenError::new(e.to_string()))? };
        let v_gep = unsafe { self.builder.build_gep(i64_ty, map_ptr, &[vs], "mfe_vp")
            .map_err(|e| CodeGenError::new(e.to_string()))? };
        let k_val = self.builder.build_load(i64_ty, k_gep, "mfe_k")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let v_val = self.builder.build_load(i64_ty, v_gep, "mfe_v")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();

        let call_args: Vec<inkwell::values::BasicMetadataValueEnum> = if is_fat {
            vec![k_val.into(), v_val.into(), cl_ptr.into()]
        } else {
            vec![k_val.into(), v_val.into()]
        };
        self.builder.build_indirect_call(fn_type, fn_p, &call_args, "mfe_call")
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let idx_n = self.builder.build_int_add(idx, one, "mfe_in")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_a, idx_n)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(exit_bb);
        Ok(())
    }

    fn build_list_distinct(
        &mut self,
        src_ptr : inkwell::values::PointerValue<'ctx>,
    ) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        let i64_ty     = self.ctx.i64_type();
        let ptr_ty     = self.ctx.ptr_type(AddressSpace::default());
        let len_fn     = self.module.get_function("arc_list_length").unwrap();
        let get_fn     = self.module.get_function("arc_list_get").unwrap();
        let new_fn     = self.module.get_function("arc_list_new").unwrap();
        let append_fn  = self.module.get_function("arc_list_append").unwrap();

        let res_call = self.builder.build_call(new_fn, &[], "dist_res")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let res_ptr = match res_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(ptr_ty.const_null()),
        };
        let src_len_call = self.builder.build_call(len_fn, &[src_ptr.into()], "dist_slen")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let src_len = match src_len_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => return Ok(res_ptr),
        };

        let cur_fn = self.cur_fn.unwrap();

        let found_alloca = self.builder.build_alloca(i64_ty, "dist_found")
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let i_alloca = self.builder.build_alloca(i64_ty, "dist_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(i_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let outer_cond = self.ctx.append_basic_block(cur_fn, "dist.outer_cond");
        let outer_body = self.ctx.append_basic_block(cur_fn, "dist.outer_body");
        let outer_exit = self.ctx.append_basic_block(cur_fn, "dist.outer_exit");

        self.builder.build_unconditional_branch(outer_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(outer_cond);
        let i = self.builder.build_load(i64_ty, i_alloca, "dist_iv")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let outer_ok = self.builder.build_int_compare(inkwell::IntPredicate::SLT, i, src_len, "dist_oc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(outer_ok, outer_body, outer_exit)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(outer_body);
        let item_call = self.builder.build_call(get_fn, &[src_ptr.into(), i.into()], "dist_item")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let item = match item_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => i64_ty.const_int(0, false),
        };

        self.builder.build_store(found_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let res_len_call = self.builder.build_call(len_fn, &[res_ptr.into()], "dist_rlen")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let res_len = match res_len_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => i64_ty.const_int(0, false),
        };

        let j_alloca = self.builder.build_alloca(i64_ty, "dist_j")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(j_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let inner_cond = self.ctx.append_basic_block(cur_fn, "dist.inner_cond");
        let inner_body = self.ctx.append_basic_block(cur_fn, "dist.inner_body");
        let inner_exit = self.ctx.append_basic_block(cur_fn, "dist.inner_exit");
        let maybe_add  = self.ctx.append_basic_block(cur_fn, "dist.maybe_add");
        let outer_inc  = self.ctx.append_basic_block(cur_fn, "dist.outer_inc");

        self.builder.build_unconditional_branch(inner_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(inner_cond);
        let j = self.builder.build_load(i64_ty, j_alloca, "dist_jv")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let found_v = self.builder.build_load(i64_ty, found_alloca, "dist_fv")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let j_ok = self.builder.build_int_compare(inkwell::IntPredicate::SLT, j, res_len, "dist_jok")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let not_found = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ, found_v, i64_ty.const_int(0, false), "dist_nf"
        ).map_err(|e| CodeGenError::new(e.to_string()))?;
        let inner_ok = self.builder.build_and(j_ok, not_found, "dist_ij_ok")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(inner_ok, inner_body, inner_exit)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(inner_body);
        let ritem_call = self.builder.build_call(get_fn, &[res_ptr.into(), j.into()], "dist_ri")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let ritem = match ritem_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::IntValue(v)) => v,
            _ => i64_ty.const_int(u64::MAX, false),
        };
        let is_dup = self.builder.build_int_compare(inkwell::IntPredicate::EQ, ritem, item, "dist_dup")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let dup_as_i64 = self.builder.build_int_z_extend(is_dup, i64_ty, "dist_dup64")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let new_found = self.builder.build_or(found_v, dup_as_i64, "dist_new_found")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(found_alloca, new_found)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let j_next = self.builder.build_int_add(j, i64_ty.const_int(1, false), "dist_jinc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(j_alloca, j_next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(inner_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(inner_exit);
        self.builder.build_unconditional_branch(maybe_add)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(maybe_add);
        let found_final = self.builder.build_load(i64_ty, found_alloca, "dist_ff")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let should_add = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ, found_final, i64_ty.const_int(0, false), "dist_sa"
        ).map_err(|e| CodeGenError::new(e.to_string()))?;
        let do_add_bb  = self.ctx.append_basic_block(cur_fn, "dist.do_add");
        self.builder.build_conditional_branch(should_add, do_add_bb, outer_inc)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(do_add_bb);
        self.builder.build_call(append_fn, &[res_ptr.into(), item.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(outer_inc)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(outer_inc);
        let i_next = self.builder.build_int_add(i, i64_ty.const_int(1, false), "dist_iinc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(i_alloca, i_next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(outer_cond)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(outer_exit);
        Ok(res_ptr)
    }

    fn build_map_entries(
        &mut self,
        map_ptr : inkwell::values::PointerValue<'ctx>,
    ) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        let i64_ty = self.ctx.i64_type();
        let i8_ty  = self.ctx.i8_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());

        let list_new_fn    = self.module.get_function("arc_list_new").unwrap();
        let list_append_fn = self.module.get_function("arc_list_append").unwrap();
        let pair_new_fn    = self.module.get_function("arc_pair_new").unwrap();

        let result_call = self.builder.build_call(list_new_fn, &[], "ent_result")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let result_ptr = match result_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(ptr_ty.const_null()),
        };

        let len_gep = unsafe {
            self.builder.build_gep(i8_ty, map_ptr, &[i64_ty.const_int(0, false)], "ent_lp")
                .map_err(|e| CodeGenError::new(e.to_string()))?
        };
        let len = self.builder.build_load(i64_ty, len_gep, "ent_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();

        let cur_fn    = self.cur_fn.unwrap();
        let idx_alloca = self.builder.build_alloca(i64_ty, "ent_idx")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let cond_bb = self.ctx.append_basic_block(cur_fn, "ent.cond");
        let body_bb = self.ctx.append_basic_block(cur_fn, "ent.body");
        let exit_bb = self.ctx.append_basic_block(cur_fn, "ent.exit");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(cond_bb);
        let idx = self.builder.build_load(i64_ty, idx_alloca, "ent_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let cond = self.builder.build_int_compare(inkwell::IntPredicate::SLT, idx, len, "ent_cond")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(cond, body_bb, exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(body_bb);
        let two     = i64_ty.const_int(2, false);
        let one     = i64_ty.const_int(1, false);
        let eight   = i64_ty.const_int(8, false);
        let ki_2    = self.builder.build_int_mul(idx, two, "ent_i2")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let k_slot  = self.builder.build_int_add(ki_2, one, "ent_ks")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let k_off   = self.builder.build_int_mul(k_slot, eight, "ent_ko")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let v_slot  = self.builder.build_int_add(ki_2, two, "ent_vs")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let v_off   = self.builder.build_int_mul(v_slot, eight, "ent_vo")
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let k_gep = unsafe { self.builder.build_gep(i8_ty, map_ptr, &[k_off], "ent_kp")
            .map_err(|e| CodeGenError::new(e.to_string()))? };
        let v_gep = unsafe { self.builder.build_gep(i8_ty, map_ptr, &[v_off], "ent_vp")
            .map_err(|e| CodeGenError::new(e.to_string()))? };
        let k_val = self.builder.build_load(i64_ty, k_gep, "ent_k")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let v_val = self.builder.build_load(i64_ty, v_gep, "ent_v")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();

        let pair_call = self.builder.build_call(pair_new_fn, &[k_val.into(), v_val.into()], "ent_pair")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        if let Some(BasicValueEnum::PointerValue(pair_ptr)) = pair_call.try_as_basic_value().basic() {
            let pair_i64 = self.builder.build_ptr_to_int(pair_ptr, i64_ty, "ent_pi")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            self.builder.build_call(list_append_fn, &[result_ptr.into(), pair_i64.into()], "")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }

        let next = self.builder.build_int_add(idx, one, "ent_inc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(exit_bb);
        Ok(result_ptr)
    }

    fn build_map_keys(
        &mut self,
        map_ptr : inkwell::values::PointerValue<'ctx>,
    ) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        self.build_map_list(map_ptr, true)
    }

    fn build_map_values(
        &mut self,
        map_ptr : inkwell::values::PointerValue<'ctx>,
    ) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        self.build_map_list(map_ptr, false)
    }

    fn build_map_list(
        &mut self,
        map_ptr   : inkwell::values::PointerValue<'ctx>,
        keys_mode : bool,
    ) -> CgResult<inkwell::values::PointerValue<'ctx>> {
        let i64_ty = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());

        let list_new_fn    = self.module.get_function("arc_list_new").unwrap();
        let list_append_fn = self.module.get_function("arc_list_append").unwrap();

        let result_call = self.builder.build_call(list_new_fn, &[], "ml_res")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let result_ptr = match result_call.try_as_basic_value().basic() {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(ptr_ty.const_null()),
        };

        let len = self.builder.build_load(i64_ty, map_ptr, "ml_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();

        let cur_fn     = self.cur_fn.unwrap();
        let idx_alloca = self.builder.build_alloca(i64_ty, "ml_idx")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let cond_bb = self.ctx.append_basic_block(cur_fn, "ml.cond");
        let body_bb = self.ctx.append_basic_block(cur_fn, "ml.body");
        let exit_bb = self.ctx.append_basic_block(cur_fn, "ml.exit");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(cond_bb);
        let idx = self.builder.build_load(i64_ty, idx_alloca, "ml_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let cond = self.builder.build_int_compare(inkwell::IntPredicate::SLT, idx, len, "ml_c")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(cond, body_bb, exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(body_bb);
        let two   = i64_ty.const_int(2, false);
        let one   = i64_ty.const_int(1, false);
        let i2    = self.builder.build_int_mul(idx, two, "ml_i2")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let off   = if keys_mode {
            self.builder.build_int_add(i2, one, "ml_ko")
                .map_err(|e| CodeGenError::new(e.to_string()))?
        } else {
            self.builder.build_int_add(i2, two, "ml_vo")
                .map_err(|e| CodeGenError::new(e.to_string()))?
        };
        let slot_gep = unsafe { self.builder.build_gep(i64_ty, map_ptr, &[off], "ml_gep")
            .map_err(|e| CodeGenError::new(e.to_string()))? };
        let item_val = self.builder.build_load(i64_ty, slot_gep, "ml_item")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();

        self.builder.build_call(list_append_fn, &[result_ptr.into(), item_val.into()], "")
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let next = self.builder.build_int_add(idx, one, "ml_inc")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, next)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.position_at_end(exit_bb);
        Ok(result_ptr)
    }

    fn compile_collection_method(
        &mut self,
        object     : &Expr,
        collection : &str,
        method     : &str,
        args       : &[Expr],
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64_ty = self.ctx.i64_type();

        let obj_val = match self.compile_expr(object)? {
            Some(v) => v,
            None    => return Ok(None),
        };
        let list_ptr = match obj_val {
            BasicValueEnum::PointerValue(p) => p,
            _ => return Ok(None),
        };

        match (collection, method) {
            ("__List", "append") => {
                let raw_item = args.first()
                    .and_then(|a| self.compile_expr(a).ok().flatten());
                let item_val = raw_item
                    .map(|v| self.value_to_i64(v)).transpose()?
                    .unwrap_or_else(|| i64_ty.const_int(0, false));
                let f = self.module.get_function("arc_list_append").unwrap();
                self.builder.build_call(f, &[list_ptr.into(), item_val.into()], "")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                // retain class elements so they survive scope exit of local vars
                let elem_class = self.infer_list_elem_class(object);
                if let Some(ec) = elem_class {
                    if self.refcount_indices.contains_key(&ec)
                        && !self.manual_memory_classes.contains(&ec)
                    {
                        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                        let item_ptr = self.builder.build_int_to_ptr(item_val, ptr_ty, "app_ptr")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        let ec_owned = ec.clone();
                        self.arc_retain_ptr(item_ptr, &ec_owned)?;
                    }
                }
                Ok(None)
            }

            ("__List", "get") => {
                let idx = args.first()
                    .and_then(|a| self.compile_expr(a).ok().flatten())
                    .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) }
                                  else { Some(i64_ty.const_int(0, false)) })
                    .unwrap_or(i64_ty.const_int(0, false));
                let f = self.module.get_function("arc_list_get").unwrap();
                let r = self.builder.build_call(f, &[list_ptr.into(), idx.into()], "list_get")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let raw = r.try_as_basic_value().basic();
                let elem_is_ptr = match object {
                    Expr::Ident(n) => {
                        let from_scope = self.lookup_var(n).and_then(|s| s.elem_class.as_deref()
                            .map(|ec| matches!(ec, "String" | "Str")
                                 || self.struct_types.contains_key(ec))
                        ).unwrap_or(false);
                        let from_param = self.param_elem_map.get(n.as_str())
                            .map(|ec| matches!(ec.as_str(), "String" | "Str")
                                 || self.struct_types.contains_key(ec.as_str()))
                            .unwrap_or(false);
                        from_scope || from_param
                    }
                    Expr::FieldAccess { object: inner, field } => {
                        let cn = if matches!(inner.as_ref(), Expr::This) {
                            self.cur_class.clone()
                        } else {
                            self.infer_object_class(inner)
                        };
                        cn.and_then(|owner| {
                            self.field_elem_classes.get(&owner)
                                .and_then(|m| m.get(field.as_str()))
                                .map(|ec| matches!(ec.as_str(), "String" | "Str")
                                     || self.struct_types.contains_key(ec.as_str()))
                        }).unwrap_or(false)
                    }
                    _ => false,
                };
                if elem_is_ptr {
                    if let Some(BasicValueEnum::IntValue(iv)) = raw {
                        let ptr = self.builder.build_int_to_ptr(iv, ptr_ty, "list_get_ptr")
                            .map_err(|e| CodeGenError::new(e.to_string()))?;
                        // retain so callers can safely release without double-free
                        let elem_class = self.infer_list_elem_class(object);
                        if let Some(ec) = elem_class {
                            if self.refcount_indices.contains_key(&ec)
                                && !self.manual_memory_classes.contains(&ec)
                            {
                                let ec_owned = ec.clone();
                                self.arc_retain_ptr(ptr, &ec_owned)?;
                            }
                        }
                        return Ok(Some(ptr.into()));
                    }
                }
                return Ok(raw);
            }

            ("__List", "set") => {
                let idx = args.first()
                    .and_then(|a| self.compile_expr(a).ok().flatten())
                    .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) }
                                  else { Some(i64_ty.const_int(0, false)) })
                    .unwrap_or(i64_ty.const_int(0, false));
                let val = args.get(1)
                    .and_then(|a| self.compile_expr(a).ok().flatten())
                    .map(|v| self.value_to_i64(v)).transpose()?
                    .unwrap_or(i64_ty.const_int(0, false));
                let f = self.module.get_function("arc_list_set").unwrap();
                self.builder.build_call(f, &[list_ptr.into(), idx.into(), val.into()], "")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                return Ok(None);
            }

            ("__List", "removeAt") => {
                let idx = args.first()
                    .and_then(|a| self.compile_expr(a).ok().flatten())
                    .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) }
                                  else { Some(i64_ty.const_int(0, false)) })
                    .unwrap_or(i64_ty.const_int(0, false));
                let len_f = self.module.get_function("arc_list_length").unwrap();
                let get_f = self.module.get_function("arc_list_get").unwrap();
                let set_f = self.module.get_function("arc_list_set").unwrap();
                let len_call = self.builder.build_call(len_f, &[list_ptr.into()], "rm_len")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let len = match len_call.try_as_basic_value().basic() {
                    Some(BasicValueEnum::IntValue(v)) => v,
                    _ => return Ok(None),
                };
                let cur_fn2 = self.cur_fn.unwrap();
                let rm_i   = self.builder.build_alloca(i64_ty, "rm_i")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_store(rm_i, idx)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let lm1    = self.builder.build_int_sub(len, i64_ty.const_int(1, false), "rm_lm1")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let rm_cond = self.ctx.append_basic_block(cur_fn2, "rm.cond");
                let rm_body = self.ctx.append_basic_block(cur_fn2, "rm.body");
                let rm_exit = self.ctx.append_basic_block(cur_fn2, "rm.exit");
                self.builder.build_unconditional_branch(rm_cond)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.position_at_end(rm_cond);
                let ri = self.builder.build_load(i64_ty, rm_i, "rm_iv")
                    .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
                let rm_ok = self.builder.build_int_compare(inkwell::IntPredicate::SLT, ri, lm1, "rm_ok")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_conditional_branch(rm_ok, rm_body, rm_exit)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.position_at_end(rm_body);
                let ri2  = self.builder.build_load(i64_ty, rm_i, "rm_iv2")
                    .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
                let ri1  = self.builder.build_int_add(ri2, i64_ty.const_int(1, false), "rm_i1")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let next_elem = self.builder.build_call(get_f, &[list_ptr.into(), ri1.into()], "rm_ne")
                    .map_err(|e| CodeGenError::new(e.to_string()))?
                    .try_as_basic_value().basic()
                    .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) } else { None })
                    .unwrap_or(i64_ty.const_int(0, false));
                self.builder.build_call(set_f, &[list_ptr.into(), ri2.into(), next_elem.into()], "")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                let ri_n = self.builder.build_int_add(ri2, i64_ty.const_int(1, false), "rm_in")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_store(rm_i, ri_n)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_unconditional_branch(rm_cond)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.position_at_end(rm_exit);
                let cur_len = self.builder.build_load(i64_ty, list_ptr, "rm_cl")
                    .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
                let new_len = self.builder.build_int_sub(cur_len, i64_ty.const_int(1, false), "rm_nl")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_store(list_ptr, new_len)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                return Ok(None);
            }

            ("__List", "length") => {
                let f = self.module.get_function("arc_list_length").unwrap();
                let r = self.builder.build_call(f, &[list_ptr.into()], "list_len")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(r.try_as_basic_value().basic())
            }

            ("__List", "isEmpty") => {
                let f = self.module.get_function("arc_list_length").unwrap();
                let r = self.builder.build_call(f, &[list_ptr.into()], "list_len")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                if let Some(BasicValueEnum::IntValue(len)) = r.try_as_basic_value().basic() {
                    let zero = i64_ty.const_int(0, false);
                    let eq = self.builder.build_int_compare(
                        inkwell::IntPredicate::EQ, len, zero, "is_empty"
                    ).map_err(|e| CodeGenError::new(e.to_string()))?;
                    return Ok(Some(eq.into()));
                }
                Ok(None)
            }

            ("__List", "filter") => {
                let elem_cls = self.infer_elem_class(object);
                let lambda_expr = args.first();
                if let Some(lambda) = lambda_expr {
                    let fn_ptr = self.compile_lambda_for_filter(lambda, elem_cls)?;
                    if let Some(fn_ptr_val) = fn_ptr {
                        let f = self.module.get_function("arc_list_filter").unwrap();
                        let r = self.builder.build_call(
                            f,
                            &[list_ptr.into(), fn_ptr_val.into()],
                            "filtered"
                        ).map_err(|e| CodeGenError::new(e.to_string()))?;
                        return Ok(r.try_as_basic_value().basic());
                    }
                }
                let f = self.module.get_function("arc_list_new").unwrap();
                let r = self.builder.build_call(f, &[], "empty_list")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(r.try_as_basic_value().basic())
            }

            ("__List", "take") | ("__List", "takeLast") => {
                let n_val = args.first().and_then(|a| self.compile_expr(a).ok().flatten())
                    .and_then(|v| if let BasicValueEnum::IntValue(iv) = v { Some(iv) } else { None })
                    .unwrap_or_else(|| i64_ty.const_int(0, false));
                let result = self.build_list_take(list_ptr, n_val, collection == "__List" && method == "takeLast")?;
                Ok(Some(result.into()))
            }

            ("__List", "map") => {
                let lambda_expr = args.first();
                if let Some(lambda) = lambda_expr {
                    if let Some(fn_ptr_val) = self.compile_expr(lambda)? {
                        let result = self.build_list_map(list_ptr, fn_ptr_val)?;
                        return Ok(Some(result.into()));
                    }
                }
                let f = self.module.get_function("arc_list_new").unwrap();
                let r = self.builder.build_call(f, &[], "empty").map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(r.try_as_basic_value().basic())
            }

            ("__List", "any") => {
                let lambda_expr = args.first();
                if let Some(lambda) = lambda_expr {
                    if let Some(fn_ptr_val) = self.compile_expr(lambda)? {
                        let result = self.build_list_any_all(list_ptr, fn_ptr_val, true)?;
                        return Ok(Some(result.into()));
                    }
                }
                Ok(Some(self.ctx.bool_type().const_int(0, false).into()))
            }

            ("__List", "all") => {
                let lambda_expr = args.first();
                if let Some(lambda) = lambda_expr {
                    if let Some(fn_ptr_val) = self.compile_expr(lambda)? {
                        let result = self.build_list_any_all(list_ptr, fn_ptr_val, false)?;
                        return Ok(Some(result.into()));
                    }
                }
                Ok(Some(self.ctx.bool_type().const_int(1, false).into()))
            }

            ("__List", "reduce") => {
                let init_val = args.first().and_then(|a| self.compile_expr(a).ok().flatten())
                    .map(|v| self.value_to_i64(v)).transpose()?
                    .unwrap_or_else(|| i64_ty.const_int(0, false));
                let lambda_expr = args.get(1);
                if let Some(lambda) = lambda_expr {
                    if let Some(fn_ptr_val) = self.compile_expr(lambda)? {
                        let result = self.build_list_reduce(list_ptr, init_val, fn_ptr_val)?;
                        return Ok(Some(result.into()));
                    }
                }
                Ok(Some(init_val.into()))
            }

            ("__List", "sortedBy") => {
                let lambda_expr = args.first();
                if let Some(lambda) = lambda_expr {
                    if let Some(fn_ptr_val) = self.compile_expr(lambda)? {
                        let result = self.build_list_sorted_by(list_ptr, fn_ptr_val)?;
                        return Ok(Some(result.into()));
                    }
                }
                Ok(Some(list_ptr.into()))
            }

            ("__List", "flatMap") => {
                let lambda_expr = args.first();
                if let Some(lambda) = lambda_expr {
                    if let Some(fn_ptr_val) = self.compile_expr(lambda)? {
                        let result = self.build_list_flat_map(list_ptr, fn_ptr_val)?;
                        return Ok(Some(result.into()));
                    }
                }
                Ok(Some(list_ptr.into()))
            }

            ("__List", "distinct") => {
                let result = self.build_list_distinct(list_ptr)?;
                Ok(Some(result.into()))
            }

            ("__List", "joinToString") => {
                for a in args { self.compile_expr(a)?; }
                let empty = self.build_global_string("")?;
                Ok(Some(empty.into()))
            }

            ("__HashMap", "set") => {
                let key = if let Some(a) = args.first() {
                    self.compile_expr(a)?
                        .map(|v| if let BasicValueEnum::PointerValue(p) = v { p }
                             else { ptr_ty.const_null() })
                        .unwrap_or(ptr_ty.const_null())
                } else { ptr_ty.const_null() };

                let val = if let Some(a) = args.get(1) {
                    self.compile_expr(a)?.map(|v| self.value_to_i64(v)).transpose()?
                        .unwrap_or_else(|| i64_ty.const_int(0, false))
                } else { i64_ty.const_int(0, false) };

                let f = self.module.get_function("arc_map_set").unwrap();
                self.builder.build_call(f, &[list_ptr.into(), key.into(), val.into()], "")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(None)
            }

            ("__HashMap", "getOrDefault") => {
                let key = if let Some(a) = args.first() {
                    self.compile_expr(a)?
                        .map(|v| if let BasicValueEnum::PointerValue(p) = v { p }
                             else { ptr_ty.const_null() })
                        .unwrap_or(ptr_ty.const_null())
                } else { ptr_ty.const_null() };

                let def = if let Some(a) = args.get(1) {
                    self.compile_expr(a)?.map(|v| self.value_to_i64(v)).transpose()?
                        .unwrap_or_else(|| i64_ty.const_int(0, false))
                } else { i64_ty.const_int(0, false) };

                let f = self.module.get_function("arc_map_get_or_default").unwrap();
                let r = self.builder.build_call(
                    f, &[list_ptr.into(), key.into(), def.into()], "map_val"
                ).map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(r.try_as_basic_value().basic())
            }

            ("__HashMap", "get") => {
                let key = if let Some(a) = args.first() {
                    self.compile_expr(a)?
                        .map(|v| if let BasicValueEnum::PointerValue(p) = v { p }
                             else { ptr_ty.const_null() })
                        .unwrap_or(ptr_ty.const_null())
                } else { ptr_ty.const_null() };
                let def = i64_ty.const_int(0, false);
                let f = self.module.get_function("arc_map_get_or_default").unwrap();
                let r = self.builder.build_call(
                    f, &[list_ptr.into(), key.into(), def.into()], "map_val"
                ).map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(r.try_as_basic_value().basic())
            }

            ("__HashMap", "containsKey") => {
                let key = if let Some(a) = args.first() {
                    self.compile_expr(a)?
                        .map(|v| if let BasicValueEnum::PointerValue(p) = v { p }
                             else { ptr_ty.const_null() })
                        .unwrap_or(ptr_ty.const_null())
                } else { ptr_ty.const_null() };
                let def = i64_ty.const_int(u64::MAX, false);
                let f = self.module.get_function("arc_map_get_or_default").unwrap();
                let r = self.builder.build_call(f, &[list_ptr.into(), key.into(), def.into()], "ck_val")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                if let Some(BasicValueEnum::IntValue(v)) = r.try_as_basic_value().basic() {
                    let found = self.builder.build_int_compare(
                        inkwell::IntPredicate::NE, v, i64_ty.const_int(u64::MAX, false), "ck_found"
                    ).map_err(|e| CodeGenError::new(e.to_string()))?;
                    return Ok(Some(found.into()));
                }
                Ok(Some(self.ctx.bool_type().const_int(0, false).into()))
            }

            ("__HashMap", "length") => {
                let i8_ty = self.ctx.i8_type();
                let gep = unsafe {
                    self.builder.build_gep(i8_ty, list_ptr,
                        &[i64_ty.const_int(0, false)], "map_len_gep")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                };
                let len_val = self.builder.build_load(i64_ty, gep, "map_len")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(Some(len_val))
            }

            ("__HashMap", "entries") => {
                let result = self.build_map_entries(list_ptr)?;
                Ok(Some(result.into()))
            }

            ("__HashMap", "keys") => {
                let result = self.build_map_keys(list_ptr)?;
                Ok(Some(result.into()))
            }

            ("__HashMap", "values") => {
                let result = self.build_map_values(list_ptr)?;
                Ok(Some(result.into()))
            }

            ("__HashMap", "remove") => {
                let key = if let Some(a) = args.first() {
                    self.compile_expr(a)?
                        .map(|v| if let BasicValueEnum::PointerValue(p) = v { p }
                             else { ptr_ty.const_null() })
                        .unwrap_or(ptr_ty.const_null())
                } else { ptr_ty.const_null() };
                self.build_map_remove(list_ptr, key)?;
                Ok(None)
            }

            ("__HashMap", "forEach") => {
                let lambda_expr = args.first();
                if let Some(lambda) = lambda_expr {
                    if let Some(fn_ptr_val) = self.compile_expr(lambda)? {
                        self.build_map_foreach(list_ptr, fn_ptr_val)?;
                    }
                }
                Ok(None)
            }

            ("__Pair", "getFirst") => {
                let f = self.module.get_function("arc_pair_first").unwrap();
                let r = self.builder.build_call(f, &[list_ptr.into()], "pair_fst")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                if let Some(BasicValueEnum::IntValue(v)) = r.try_as_basic_value().basic() {
                    let ptr = self.builder.build_int_to_ptr(v, ptr_ty, "fst_ptr")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    return Ok(Some(ptr.into()));
                }
                Ok(r.try_as_basic_value().basic())
            }

            ("__Pair", "getSecond") => {
                let f = self.module.get_function("arc_pair_second").unwrap();
                let r = self.builder.build_call(f, &[list_ptr.into()], "pair_snd")
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                Ok(r.try_as_basic_value().basic())
            }

            _ => {
                for a in args { self.compile_expr(a)?; }
                Ok(None)
            }
        }
    }

    fn compile_lambda_for_filter(
        &mut self,
        lambda   : &Expr,
        elem_cls : Option<String>,
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        let (params, body) = match lambda {
            Expr::Lambda { params, body } => (params.clone(), body.clone()),
            _ => return Ok(None),
        };

        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64_ty = self.ctx.i64_type();
        let i1_ty  = self.ctx.bool_type();

        self.lambda_counter += 1;
        let fn_name = format!("arc_lambda_{}", self.lambda_counter);

        let fn_ty = i64_ty.fn_type(&[i64_ty.into()], false);
        let fn_val = self.module.add_function(&fn_name, fn_ty, None);

        let prev_block = self.builder.get_insert_block();
        let prev_fn = self.cur_fn;
        let prev_class = self.cur_class.clone();

        let entry = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);
        self.cur_fn = Some(fn_val);
        self.push_scope();

        let param_name = params.first().map(|s| s.as_str()).unwrap_or("item");
        let item_i64 = fn_val.get_nth_param(0).unwrap().into_int_value();

        let item_ptr = self.builder.build_int_to_ptr(item_i64, ptr_ty, "item_ptr")
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let alloca = self.builder.build_alloca(ptr_ty, param_name)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(alloca, item_ptr)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let ec = elem_cls.clone();
        self.define_collection_var(param_name, alloca, ptr_ty.into(), ec.clone(), None);

        if let Some(ref cls) = ec {
            self.cur_class = Some(cls.clone());
        }

        let result = self.compile_expr(&body)?;

        let ret_val = match result {
            Some(BasicValueEnum::IntValue(v)) => {
                if v.get_type().get_bit_width() == 1 {
                    self.builder.build_int_z_extend(v, i64_ty, "zext_ret")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                } else if v.get_type().get_bit_width() < 64 {
                    self.builder.build_int_z_extend(v, i64_ty, "zext_ret")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                } else {
                    v
                }
            }
            Some(BasicValueEnum::PointerValue(p)) => {
                self.builder.build_ptr_to_int(p, i64_ty, "ptr2i")
                    .map_err(|e| CodeGenError::new(e.to_string()))?
            }
            _ => i64_ty.const_int(0, false),
        };

        let _ = i1_ty;
        self.builder.build_return(Some(&ret_val))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.pop_scope();
        self.cur_fn = prev_fn;
        self.cur_class = prev_class;

        if let Some(bb) = prev_block {
            self.builder.position_at_end(bb);
        }

        fn_val.verify(true);

        Ok(Some(fn_val.as_global_value().as_pointer_value().into()))
    }

    fn collect_idents_in_expr(expr: &Expr, out: &mut Vec<String>) {
        match expr {
            Expr::Ident(n) => out.push(n.clone()),
            Expr::BinOp { left, right, .. } => {
                Self::collect_idents_in_expr(left, out);
                Self::collect_idents_in_expr(right, out);
            }
            Expr::UnaryOp { expr, .. } => Self::collect_idents_in_expr(expr, out),
            Expr::Cast { expr, .. }    => Self::collect_idents_in_expr(expr, out),
            Expr::Await(e)             => Self::collect_idents_in_expr(e, out),
            Expr::MethodCall { object, args, .. } | Expr::NullSafeAccess { object, args: Some(args), .. } => {
                Self::collect_idents_in_expr(object, out);
                for a in args { Self::collect_idents_in_expr(a, out); }
            }
            Expr::NullSafeAccess { object, args: None, .. } => Self::collect_idents_in_expr(object, out),
            Expr::StaticCall { args, .. } | Expr::ConstructorCall { args, .. } => {
                for a in args { Self::collect_idents_in_expr(a, out); }
            }
            Expr::Ternary { cond, then, else_ } => {
                Self::collect_idents_in_expr(cond, out);
                Self::collect_idents_in_expr(then, out);
                Self::collect_idents_in_expr(else_, out);
            }
            Expr::FieldAccess { object, .. } => Self::collect_idents_in_expr(object, out),
            Expr::StrInterp(parts) => {
                for p in parts {
                    if let StringPart::Interp(e) = p { Self::collect_idents_in_expr(e, out); }
                }
            }
            Expr::Lambda { params: inner_params, body } => {
                let mut inner = Vec::new();
                Self::collect_idents_in_expr(body, &mut inner);
                let inner_set: std::collections::HashSet<_> = inner_params.iter().cloned().collect();
                for n in inner { if !inner_set.contains(&n) { out.push(n); } }
            }
            Expr::Match { expr, arms } => {
                Self::collect_idents_in_expr(expr, out);
                for arm in arms {
                    if let Some(g) = &arm.guard { Self::collect_idents_in_expr(g, out); }
                    Self::collect_idents_in_expr(&arm.body, out);
                }
            }
            Expr::NullCoalesce { left, right } => {
                Self::collect_idents_in_expr(left, out);
                Self::collect_idents_in_expr(right, out);
            }
            _ => {}
        }
    }

    fn find_free_vars(&self, body: &Expr, params: &[String]) -> Vec<String> {
        let mut all_idents = Vec::new();
        Self::collect_idents_in_expr(body, &mut all_idents);
        let param_set: std::collections::HashSet<_> = params.iter().cloned().collect();
        let mut free = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for name in all_idents {
            if !param_set.contains(&name) && !seen.contains(&name) && self.lookup_var(&name).is_some() {
                free.push(name.clone());
                seen.insert(name);
            }
        }
        free
    }

    fn compile_general_lambda(
        &mut self,
        params : &[String],
        body   : &Expr,
    ) -> CgResult<Option<BasicValueEnum<'ctx>>> {
        let i64_ty = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());

        self.lambda_counter += 1;
        let counter = self.lambda_counter;
        let fn_name = format!("arc_lambda_{}", counter);

        let free_vars = self.find_free_vars(body, params);

        let mut param_types: Vec<inkwell::types::BasicMetadataTypeEnum> =
            params.iter().map(|_| i64_ty.into()).collect();
        param_types.push(ptr_ty.into());
        let fn_ty  = i64_ty.fn_type(&param_types, false);
        let fn_val = self.module.add_function(&fn_name, fn_ty, None);

        let prev_block = self.builder.get_insert_block();
        let prev_fn    = self.cur_fn;
        let prev_class = self.cur_class.clone();

        let entry = self.ctx.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry);
        self.cur_fn = Some(fn_val);
        self.push_scope();

        for (i, param_name) in params.iter().enumerate() {
            if let Some(pv) = fn_val.get_nth_param(i as u32) {
                let alloca = self.builder.build_alloca(i64_ty, param_name)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.builder.build_store(alloca, pv)
                    .map_err(|e| CodeGenError::new(e.to_string()))?;
                self.define_var(param_name, alloca, i64_ty.into());
            }
        }

        if !free_vars.is_empty() {
            let closure_param_idx = params.len() as u32;
            if let Some(closure_param) = fn_val.get_nth_param(closure_param_idx) {
                let closure_ptr = closure_param.into_pointer_value();
                let field_types: Vec<BasicTypeEnum<'ctx>> =
                    free_vars.iter().map(|_| i64_ty.as_basic_type_enum()).collect();
                let cty = self.ctx.struct_type(&field_types, false);
                for (i, var_name) in free_vars.iter().enumerate() {
                    let gep = self.builder.build_struct_gep(cty, closure_ptr, i as u32, "cap_gep")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let cap_val = self.builder.build_load(i64_ty, gep, var_name)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let alloca = self.builder.build_alloca(i64_ty, var_name)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_store(alloca, cap_val)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.define_var(var_name, alloca, i64_ty.into());
                }
            }
        }

        let result  = self.compile_expr(body)?;
        let ret_val = match result {
            Some(BasicValueEnum::IntValue(v)) => {
                if v.get_type().get_bit_width() < 64 {
                    self.builder.build_int_z_extend(v, i64_ty, "lz")
                        .map_err(|e| CodeGenError::new(e.to_string()))?
                } else { v }
            }
            Some(BasicValueEnum::FloatValue(f)) => {
                self.builder.build_float_to_signed_int(f, i64_ty, "f2i")
                    .map_err(|e| CodeGenError::new(e.to_string()))?
            }
            Some(BasicValueEnum::PointerValue(p)) => {
                self.builder.build_ptr_to_int(p, i64_ty, "p2i")
                    .map_err(|e| CodeGenError::new(e.to_string()))?
            }
            _ => i64_ty.const_int(0, false),
        };
        self.builder.build_return(Some(&ret_val))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.pop_scope_no_arc();
        self.cur_fn    = prev_fn;
        self.cur_class = prev_class;

        if let Some(bb) = prev_block {
            self.builder.position_at_end(bb);
        }

        let closure_ptr = if !free_vars.is_empty() {
            let field_types: Vec<BasicTypeEnum<'ctx>> =
                free_vars.iter().map(|_| i64_ty.as_basic_type_enum()).collect();
            let cty     = self.ctx.struct_type(&field_types, false);
            self.declare_malloc();
            let malloc  = self.module.get_function("malloc").unwrap();
            let size    = i64_ty.const_int((free_vars.len() as u64) * 8, false);
            let alloc_call = self.builder.build_call(malloc, &[size.into()], "closure_alloc")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            let alloc_ptr = match alloc_call.try_as_basic_value().basic() {
                Some(BasicValueEnum::PointerValue(p)) => p,
                _ => return Err(CodeGenError::new("closure malloc void")),
            };
            for (i, var_name) in free_vars.iter().enumerate() {
                if let Some(slot) = self.lookup_var(var_name) {
                    let slot = slot.clone();
                    let val = self.builder.build_load(slot.ty, slot.ptr, "cap_v")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    let val_i64 = self.value_to_i64(val)?;
                    let gep = self.builder.build_struct_gep(cty, alloc_ptr, i as u32, "cap_gep")
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                    self.builder.build_store(gep, val_i64)
                        .map_err(|e| CodeGenError::new(e.to_string()))?;
                }
            }
            alloc_ptr
        } else {
            ptr_ty.const_null()
        };

        let fat_ty = self.ctx.struct_type(&[ptr_ty.into(), ptr_ty.into()], false);
        let fat_alloca = self.builder.build_alloca(fat_ty, "fat_ptr")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let gep0 = self.builder.build_struct_gep(fat_ty, fat_alloca, 0, "fat_fn")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(gep0, fn_val.as_global_value().as_pointer_value())
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let gep1 = self.builder.build_struct_gep(fat_ty, fat_alloca, 1, "fat_cl")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(gep1, closure_ptr)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let fat = self.builder.build_load(fat_ty, fat_alloca, "fat")
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        fn_val.verify(true);
        Ok(Some(fat))
    }

    fn extract_fn_closure(
        &mut self,
        val : BasicValueEnum<'ctx>,
    ) -> CgResult<(PointerValue<'ctx>, PointerValue<'ctx>)> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        match val {
            BasicValueEnum::PointerValue(p) => Ok((p, ptr_ty.const_null())),
            BasicValueEnum::StructValue(s)  => {
                let fn_p = self.builder.build_extract_value(s, 0, "fat_fn")
                    .map_err(|e| CodeGenError::new(e.to_string()))?
                    .into_pointer_value();
                let cl_p = self.builder.build_extract_value(s, 1, "fat_cl")
                    .map_err(|e| CodeGenError::new(e.to_string()))?
                    .into_pointer_value();
                Ok((fn_p, cl_p))
            }
            _ => Err(CodeGenError::new("expected fn ptr or fat ptr")),
        }
    }

    fn compile_for_each(
        &mut self,
        ty   : &Type,
        name : &str,
        iter : &Expr,
        body : &[Stmt],
    ) -> CgResult<bool> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64_ty = self.ctx.i64_type();

        let iter_val = match self.compile_expr(iter)? {
            Some(BasicValueEnum::PointerValue(p)) => p,
            _ => return Ok(false),
        };

        let list_len_fn = self.module.get_function("arc_list_length").unwrap();
        let list_get_fn = self.module.get_function("arc_list_get").unwrap();

        let len_call = self.builder.build_call(list_len_fn, &[iter_val.into()], "fe_len")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let len_val = len_call.try_as_basic_value().basic()
            .ok_or_else(|| CodeGenError::new("arc_list_length returned void"))?;
        let len_i64 = match len_val { BasicValueEnum::IntValue(v) => v, _ => return Ok(false) };

        let idx_alloca = self.builder.build_alloca(i64_ty, "fe_idx")
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_store(idx_alloca, i64_ty.const_int(0, false))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let cur_fn = self.cur_fn.unwrap();
        let cond_bb = self.ctx.append_basic_block(cur_fn, "fe.cond");
        let body_bb = self.ctx.append_basic_block(cur_fn, "fe.body");
        let exit_bb = self.ctx.append_basic_block(cur_fn, "fe.exit");

        self.builder.build_unconditional_branch(cond_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(cond_bb);
        let idx = self.builder.build_load(BasicTypeEnum::IntType(i64_ty), idx_alloca, "fe_i")
            .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
        let cond = self.builder.build_int_compare(
            inkwell::IntPredicate::SLT, idx, len_i64, "fe_cond"
        ).map_err(|e| CodeGenError::new(e.to_string()))?;
        self.builder.build_conditional_branch(cond, body_bb, exit_bb)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        self.builder.position_at_end(body_bb);
        self.push_scope();

        let item_call = self.builder.build_call(
            list_get_fn, &[iter_val.into(), idx.into()], "fe_item"
        ).map_err(|e| CodeGenError::new(e.to_string()))?;

        if let Some(BasicValueEnum::IntValue(item_i64)) = item_call.try_as_basic_value().basic() {
            let item_ptr = self.builder.build_int_to_ptr(item_i64, ptr_ty, "fe_ptr")
                .map_err(|e| CodeGenError::new(e.to_string()))?;

            let alloca = self.builder.build_alloca(ptr_ty, name)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            self.builder.build_store(alloca, item_ptr)
                .map_err(|e| CodeGenError::new(e.to_string()))?;

            let class_name = match ty {
                Type::Named(n) => Some(n.clone()),
                _ => None,
            };
            self.define_collection_var(name, alloca, ptr_ty.into(), class_name, None);
        }

        for s in body {
            if self.compile_stmt(s)? { break; }
        }
        self.pop_scope();

        if !self.current_block_terminated() {
            let idx2 = self.builder.build_load(BasicTypeEnum::IntType(i64_ty), idx_alloca, "fe_i2")
                .map_err(|e| CodeGenError::new(e.to_string()))?.into_int_value();
            let idx3 = self.builder.build_int_add(idx2, i64_ty.const_int(1, false), "fe_inc")
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            self.builder.build_store(idx_alloca, idx3)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
            self.builder.build_unconditional_branch(cond_bb)
                .map_err(|e| CodeGenError::new(e.to_string()))?;
        }

        self.builder.position_at_end(exit_bb);
        Ok(false)
    }

    fn value_to_i64(
        &mut self,
        val : BasicValueEnum<'ctx>,
    ) -> CgResult<inkwell::values::IntValue<'ctx>> {
        let i64_ty = self.ctx.i64_type();
        match val {
            BasicValueEnum::IntValue(v) => {
                let w = v.get_type().get_bit_width();
                if w < 64 {
                    self.builder.build_int_z_extend(v, i64_ty, "zext64")
                        .map_err(|e| CodeGenError::new(e.to_string()))
                } else if w > 64 {
                    self.builder.build_int_truncate(v, i64_ty, "trunc64")
                        .map_err(|e| CodeGenError::new(e.to_string()))
                } else {
                    Ok(v)
                }
            }
            BasicValueEnum::PointerValue(p) => {
                self.builder.build_ptr_to_int(p, i64_ty, "ptr2i64")
                    .map_err(|e| CodeGenError::new(e.to_string()))
            }
            BasicValueEnum::FloatValue(f) => {
                self.builder.build_float_to_signed_int(f, i64_ty, "f2i64")
                    .map_err(|e| CodeGenError::new(e.to_string()))
            }
            _ => Ok(i64_ty.const_int(0, false)),
        }
    }

    pub fn emit_ir(&self) -> String {
        self.module.print_to_string().to_string()
    }

    pub fn emit_object_file(&self, path: &Path) -> CgResult<()> {
        self.emit_object_file_opts(path, false)
    }

    pub fn emit_object_file_opts(&self, path: &Path, optimize: bool) -> CgResult<()> {
        Target::initialize_x86(&InitializationConfig::default());

        let triple   = TargetMachine::get_default_triple();
        let target   = Target::from_triple(&triple)
            .map_err(|e| CodeGenError::new(e.to_string()))?;
        let cpu      = TargetMachine::get_host_cpu_name();
        let features = TargetMachine::get_host_cpu_features();

        let opt_level = if optimize {
            OptimizationLevel::Aggressive
        } else {
            OptimizationLevel::Default
        };

        let machine = target.create_target_machine(
            &triple,
            cpu.to_str().unwrap_or("generic"),
            features.to_str().unwrap_or(""),
            opt_level,
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

pub fn emit_ir_only(
    module_ast  : &crate::ast::Module,
    module_name : &str,
) -> Result<String, CodeGenError> {
    let ctx = Context::create();
    let mut cg = CodeGen::new(&ctx, module_name);
    cg.compile_module(module_ast)?;
    Ok(cg.emit_ir())
}

pub fn compile_to_object(
    module_ast  : &crate::ast::Module,
    module_name : &str,
    out_path    : &Path,
) -> Result<(), CodeGenError> {
    compile_to_object_opts(module_ast, module_name, out_path, false)
}

pub fn compile_to_object_opts(
    module_ast  : &crate::ast::Module,
    module_name : &str,
    out_path    : &Path,
    optimize    : bool,
) -> Result<(), CodeGenError> {
    let ctx = Context::create();
    let mut cg = CodeGen::new(&ctx, module_name);
    cg.compile_module(module_ast)?;
    cg.verify_module()?;
    cg.emit_object_file_opts(out_path, optimize)
}

pub fn compile_to_object_multi(
    modules     : &[&crate::ast::Module],
    module_name : &str,
    out_path    : &Path,
    optimize    : bool,
) -> Result<(), CodeGenError> {
    let ctx = Context::create();
    let mut cg = CodeGen::new(&ctx, module_name);
    for m in modules {
        cg.compile_module(m)?;
    }
    cg.verify_module()?;
    cg.emit_object_file_opts(out_path, optimize)
}

pub fn emit_ir_multi(
    modules     : &[&crate::ast::Module],
    module_name : &str,
) -> Result<String, CodeGenError> {
    let ctx = Context::create();
    let mut cg = CodeGen::new(&ctx, module_name);
    for m in modules {
        cg.compile_module(m)?;
    }
    Ok(cg.emit_ir())
}
