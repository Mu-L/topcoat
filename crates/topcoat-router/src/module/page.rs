use std::borrow::Cow;

use crate::{GenerateStaticFn, LayoutFn, LayoutRenderFn, OwnedMethods, PageFn, PageRenderFn, Path};

/// A page discovered by the module router, produced by the `#[page]` macro.
///
/// Holds the module path (for deriving the URL path from the module tree)
/// and the render function. The module router converts each `ModulePageFn`
/// into a [`PageFn`] once the URL path has been computed.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct ModulePageFn {
    /// The HTTP methods this page responds to.
    methods: OwnedMethods,
    /// Module path where `#[page]` was declared, used to derive the URL path.
    module_path: &'static str,
    /// The page's async render function, returning a [`Result`].
    pub(super) render: PageRenderFn,
    /// The page's `generate_static` function, when it declares one.
    generate_static: Option<GenerateStaticFn>,
}

impl ModulePageFn {
    /// Creates a new module page. Called by the expanded `#[page]` macro.
    pub const fn new(
        methods: OwnedMethods,
        module_path: &'static str,
        render: PageRenderFn,
    ) -> Self {
        Self {
            methods,
            module_path,
            render,
            generate_static: None,
        }
    }

    /// Declares the function supplying the [`StaticParams`](crate::StaticParams)
    /// sets this page is exported for, as `#[page(generate_static = ...)]` does.
    #[must_use]
    pub const fn with_generate_static(mut self, generate_static: GenerateStaticFn) -> Self {
        self.generate_static = Some(generate_static);
        self
    }

    /// Converts into a [`PageFn`] with the given resolved URL path.
    #[must_use]
    pub fn into_page(self, path: Cow<'static, Path>) -> PageFn {
        let page = PageFn::new(self.methods, path, self.render);
        match self.generate_static {
            Some(generate_static) => page.with_generate_static(generate_static),
            None => page,
        }
    }

    /// Returns the module path used to derive the URL.
    #[must_use]
    pub fn module_path(&self) -> &'static str {
        self.module_path
    }
}

#[cfg(feature = "discover")]
inventory::collect!(ModulePageFn);

/// A layout discovered by the module router, produced by the `#[layout]` macro.
///
/// Holds the module path (for deriving the URL prefix from the module tree)
/// and the render function. The module router converts each `ModuleLayoutFn`
/// into a [`LayoutFn`] once the URL path has been computed.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct ModuleLayoutFn {
    /// Module path where `#[layout]` was declared, used to derive the URL path.
    module_path: &'static str,
    /// The layout's async render function, receiving the child content as a [`Result`] and
    /// returning a new [`Result`].
    render: LayoutRenderFn,
}

impl ModuleLayoutFn {
    /// Creates a new module layout. Called by the expanded `#[layout]` macro.
    pub const fn new(module_path: &'static str, render: LayoutRenderFn) -> Self {
        Self {
            module_path,
            render,
        }
    }

    /// Converts into a [`LayoutFn`] with the given resolved URL path.
    #[must_use]
    pub fn into_layout(self, path: Cow<'static, Path>) -> LayoutFn {
        LayoutFn::new(path, self.render)
    }

    /// Returns the module path used to derive the URL.
    #[must_use]
    pub fn module_path(&self) -> &'static str {
        self.module_path
    }
}

#[cfg(feature = "discover")]
inventory::collect!(ModuleLayoutFn);
