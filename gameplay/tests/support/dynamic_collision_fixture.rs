//! Invented shapes, independent of private original resources.
use serde_json::{Value, json};
pub fn fixture() -> Value {
    json!({"format":"mag_dynamic_bodies","schema_version":1,
 "source":{"native_binding_sha256":"a".repeat(64),"shape_data_sha256":"b".repeat(64)},
 "templates":[{"id":"invented-vehicle","kind":"ground_vehicle","roots":[
 {"id":"chassis","layer":22,"translation":[0.,0.,0.],"rotation":"entity","shape":{"kind":"hull","vertices":vec![[-1.,-1.,-1.],[-1.,-1.,1.],[-1.,1.,-1.],[-1.,1.,1.],[1.,-1.,-1.],[1.,-1.,1.],[1.,1.,-1.],[1.,1.,1.]],"radius":0.1}},
 {"id":"wheel","layer":22,"translation":[0.,-1.,0.],"rotation":"entity","shape":{"kind":"box","half_extents":[1.,0.5,1.],"radius":0.1}}]},
 {"id":"invented-character","kind":"character","roots":[{"id":"capsule","layer":21,"dead_layer":6,"rotation":"identity","translation":[0.,0.,0.],"shape":{"kind":"capsule","a":[0.,0.,0.],"b":[0.,1.,0.],"radius":0.4}}]},
 {"id":"invented-mine","kind":"mine","roots":[{"id":"sphere","layer":23,"translation":[0.,0.,0.],"rotation":"entity","shape":{"kind":"sphere","radius":0.2}}]}]})
}
