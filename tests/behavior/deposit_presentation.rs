use bevy::{mesh::VertexAttributeValues, prelude::*};
use top_down_2d_rts_prototype_nano_swarm::{
    deposit_presentation::{DepositMaterial, sync_deposit_presentation},
    resources::{ResourceDeposit, ResourceKind},
};

#[test]
fn extraction_shrinks_crystals_without_shrinking_the_blocking_base() {
    let mut app = App::new();
    app.init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<DepositMaterial>>()
        .add_systems(Update, sync_deposit_presentation);
    let entity = app
        .world_mut()
        .spawn((
            ResourceDeposit {
                kind: ResourceKind::Minerals,
                amount: 800,
                capacity: 800,
                radius: 64.0,
            },
            Sprite::default(),
            Transform::from_xyz(250.0, -100.0, 2.0).with_scale(Vec3::new(2.0, 3.0, 1.0)),
        ))
        .id();
    app.update();
    assert!(app.world().get::<Sprite>(entity).is_none());
    let material = app
        .world()
        .get::<MeshMaterial2d<DepositMaterial>>(entity)
        .unwrap()
        .0
        .clone();
    assert!(
        (app.world()
            .resource::<Assets<DepositMaterial>>()
            .get(&material)
            .unwrap()
            .appearance
            .x
            - 1.0)
            .abs()
            < 0.001
    );
    app.world_mut()
        .get_mut::<ResourceDeposit>(entity)
        .unwrap()
        .amount = 200;
    app.update();
    assert!(
        (app.world()
            .resource::<Assets<DepositMaterial>>()
            .get(&material)
            .unwrap()
            .appearance
            .x
            - 0.25)
            .abs()
            < 0.001
    );
    app.world_mut()
        .get_mut::<ResourceDeposit>(entity)
        .unwrap()
        .amount = 0;
    app.update();
    assert!(
        app.world()
            .resource::<Assets<DepositMaterial>>()
            .get(&material)
            .unwrap()
            .appearance
            .x
            .abs()
            < 0.001
    );
    let mesh_handle = &app.world().get::<Mesh2d>(entity).unwrap().0;
    let mesh = app
        .world()
        .resource::<Assets<Mesh>>()
        .get(mesh_handle)
        .unwrap();
    let VertexAttributeValues::Float32x3(positions) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
    else {
        panic!("quad positions missing")
    };
    let transform = app.world().get::<Transform>(entity).unwrap();
    for position in positions {
        assert!((position[0].abs() * transform.scale.x - 64.0).abs() < 0.001);
        assert!((position[1].abs() * transform.scale.y - 64.0).abs() < 0.001);
    }
    assert!((app.world().get::<ResourceDeposit>(entity).unwrap().radius - 64.0).abs() < 0.001);
}
