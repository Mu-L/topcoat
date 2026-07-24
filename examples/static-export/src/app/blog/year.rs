mod slug;

use topcoat::router::path_param;

/// Turns this module's segment into `{year}`, so pages below it serve
/// `/blog/{year}/...`.
#[path_param]
struct Year(str);
