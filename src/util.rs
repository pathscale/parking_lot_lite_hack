// Copyright 2016 Amanieu d'Antras
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use core::time::Duration;
use parking_lot_core::time::Instant;

#[inline]
pub fn to_deadline(timeout: Duration) -> Option<Instant> {
    Instant::now().checked_add(timeout)
}
