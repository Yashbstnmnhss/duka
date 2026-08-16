//! Object

use duka_backend::value::RuntimeValue;
use duka_lib::harness::run_last;

#[test]
fn object_property_and_instance_method() {
    let r = run_last(
        r#"
object A
    property = 1
    function :init(a)
        self.value = a
    end
    function :method(a)
        return self.value + a
    end
end
local a = A.new(10)
local b = A.new(100)
return A.property + a:method(2) + b:method(1)
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(114));
}

#[test]
fn object_empty_generates_fallback_init() {
    let r = run_last(
        r#"
object A
end
local a = A.new()
return 42
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(42));
}

#[test]
fn object_static_method() {
    let r = run_last(
        r#"
object A
    function double(x)
        return x * 2
    end
end
return A.double(21)
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(42));
}

#[test]
fn object_static_property_reassignment() {
    let r = run_last(
        r#"
object A
    count = 1
end
A.count = A.count + 10
return A.count
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(11));
}

#[test]
fn object_inherit_method_resolution() {
    let r = run_last(
        r#"
object A
    function :init(a)
        self.value = a
    end
    function :method()
        return self.value + 1
    end
end
object B extends A
    function :method()
        return self.value + 10
    end
end
local b = B.new(5)
return b:method()
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(15));
}

#[test]
fn object_inherit_class_level_property() {
    let r = run_last(
        r#"
object A
    shared = 7
end
object B extends A
end
return B.shared
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(7));
}

#[test]
fn object_inherit_manual_super_init() {
    let r = run_last(
        r#"
object A
    function :init(a)
        self.value = a
    end
end
object B extends A
    function :init(a)
        A.init(self, a + 1)
        self.extra = a * 10
    end
end
local b = B.new(5)
return b.value + b.extra
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(56));
}

#[test]
fn object_inherit_multilevel() {
    let r = run_last(
        r#"
object A
    function :init(a)
        self.value = a
    end
    function :method()
        return self.value + 1
    end
end
object B extends A
    function :method()
        return self.value + 10
    end
end
object C extends B
    function :method()
        return self.value + 100
    end
end
local c = C.new(3)
return c:method()
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(103));
}

#[test]
fn object_custom_new() {
    let r = run_last(
        r#"
object A
    function :init(v)
        self.value = v
    end
    function new(v)
        local s = set_metatable({}, A)
        s:init(v)
        return s
    end
end
local a = A.new(99)
return a.value
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(99));
}

#[test]
fn object_computed_key_property() {
    let r = run_last(
        r#"
object A
    ["k" .. "ey"] = 5
end
return A.key
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(5));
}

#[test]
fn object_method_on_class_itself() {
    let r = run_last(
        r#"
object A
    function :method(a)
        return a + 1
    end
end
return A:method(10)
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(11));
}

#[test]
fn object_new_returns_distinct_instances() {
    let r = run_last(
        r#"
object A
    function :init()
        self.n = 0
    end
    function __tostring()
        return "12312"
    end
    function :bump()
        self.n = self.n + 1
        return self.n
    end
end
local a = A.new()
local b = A.new()
print(a)
local r1 = a:bump()
local r2 = b:bump()
return r1 * 10 + r2
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(11));
}

#[test]
fn data_object_auto_init() {
    let r = run_last(
        r#"
@data object A
    x
    y
end
local a = A.new(3, 5)
return a.x + a.y
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(8));
}

#[test]
fn data_object_auto_tostring() {
    let r = run_last(
        r#"
@data object A
    x = 3
    y = 5
end
local a = A.new(3, 5)
return to_string(a)
"#,
    )
    .unwrap();
    assert_eq!(r.eval_to_string(), "A{x=3, y=5}");
}

#[test]
fn data_object_auto_tostring_empty() {
    let r = run_last(
        r#"
@data object A
end
return to_string(A.new())
"#,
    )
    .unwrap();
    assert_eq!(r.eval_to_string(), "A{}");
}

#[test]
fn data_object_auto_tostring_nil_prop() {
    let r = run_last(
        r#"
@data object A
    x
    y
end
local a = A.new(1)
return to_string(a)
"#,
    )
    .unwrap();
    assert_eq!(r.eval_to_string(), "A{x=1, y=nil}");
}

#[test]
fn object_super_init() {
    let r = run_last(
        r#"
object A
    function :init(a)
        self.value = a
    end
end
object B extends A
    function :init(a)
        super:init(a + 1)
        self.extra = a * 10
    end
end
local b = B.new(5)
return b.value + b.extra
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(56));
}

#[test]
fn object_super_method_override() {
    let r = run_last(
        r#"
object A
    function :init(a)
        self.value = a
    end
    function :method()
        return self.value + 1
    end
end
object B extends A
    function :method()
        return super:method() + 10
    end
end
local b = B.new(5)
return b:method()
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(16));
}

#[test]
fn object_super_multilevel() {
    let r = run_last(
        r#"
object A
    function :init()
        self.n = 1
    end
    function :method()
        return self.n
    end
end
object B extends A
    function :init()
        super:init()
        self.n = self.n + 1
    end
    function :method()
        return super:method() + 10
    end
end
object C extends B
    function :method()
        return super:method() + 100
    end
end
local c = C.new()
return c:method()
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(112));
}

#[test]
fn object_super_class_property() {
    let r = run_last(
        r#"
object A
    shared = 7
end
object B extends A
    function get()
        return super.shared
    end
end
return B.get()
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(7));
}

#[test]
fn object_super_dot_form() {
    let r = run_last(
        r#"
object A
    function :init(a)
        self.value = a
    end
end
object B extends A
    function :init(a)
        super.init(self, a + 1)
    end
end
return B.new(5).value
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(6));
}

#[test]
fn data_object_super_init() {
    let r = run_last(
        r#"
@data object A
    x
    y
end
@data object B extends A
    z
    function :init(x, y, z)
        super:init(x, y)
        self.z = z
    end
end
local b = B.new(1, 2, 3)
return b.x + b.y * 10 + b.z * 100
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(321));
}
