use super::*;

pub fn mesh_snake(snake: &Snake, interpolation: f32) -> Mesh {
    let mut verticies = Vec::new();

    fn push_quad(
        verticies: &mut Vec<[f32; 3]>,
        pos: IVec2,
        offset: Vec2,
        half_size: Vec2,
        flip: IVec2,
    ) {
        let offset = if flip.y == 1 {
            Vec2::new(offset.y, offset.x)
        } else {
            offset
        };

        let half_size = if flip.y == 1 {
            Vec2::new(half_size.y, half_size.x)
        } else {
            half_size
        };
        let pos = Vec2::new(pos.x as f32, pos.y as f32) + 0.5 + offset * flip.x as f32;

        verticies.push([pos.x - half_size.x, pos.y - half_size.y, 0.0]);
        verticies.push([pos.x + half_size.x, pos.y - half_size.y, 0.0]);
        verticies.push([pos.x - half_size.x, pos.y + half_size.y, 0.0]);

        verticies.push([pos.x - half_size.x, pos.y + half_size.y, 0.0]);
        verticies.push([pos.x + half_size.x, pos.y - half_size.y, 0.0]);
        verticies.push([pos.x + half_size.x, pos.y + half_size.y, 0.0]);
    }

    fn push_circle(verticies: &mut Vec<[f32; 3]>, pos: IVec2, offset: Vec2, radius: f32) {
        let pos = Vec2::new(pos.x as f32, pos.y as f32) + 0.5 + offset;

        let segments = 64;

        let step = std::f32::consts::TAU / segments as f32;
        let mut angle = step;
        let mut last = Vec2::new(0.0, radius);
        for _ in 0..segments {
            let x = radius * angle.sin();
            let y = radius * angle.cos();

            verticies.push([pos.x, pos.y, 0.0]);
            verticies.push([pos.x + x, pos.y + y, 0.0]);
            verticies.push([pos.x + last.x, pos.y + last.y, 0.0]);

            angle += step;
            last = Vec2::new(x, y);
        }
    }

    let width = 0.6;
    let head_size = 0.7;

    let head = snake.body[0];
    let neck = snake.body[1];
    let len = snake.body.len();
    let tail = snake.body[len - 1];

    let mut start = 1;
    let mut end = len - 1;

    if interpolation >= 0.0 {
        start = 0;

        // Interpolate head
        push_quad(
            &mut verticies,
            head,
            Vec2::new(0.0, interpolation / 2.0),
            Vec2::new(width / 2.0, interpolation / 2.0),
            calculate_flip(snake.head_dir),
        );
        push_circle(
            &mut verticies,
            head,
            snake.head_dir.as_vec2() * interpolation,
            head_size / 2.0,
        );

        // Interpolate tail
        let tail_dir = snake.body[len - 2] - snake.body[len - 1];
        push_quad(
            &mut verticies,
            tail,
            Vec2::new(0.0, interpolation / 2.0 + 0.25),
            Vec2::new(width / 2.0, -interpolation / 2.0 + 0.25),
            calculate_flip(tail_dir),
        );
        push_circle(
            &mut verticies,
            tail,
            tail_dir.as_vec2() * interpolation,
            width / 2.0,
        );
    } else {
        end = len;

        // Interpolate head
        push_quad(
            &mut verticies,
            head,
            Vec2::new(0.0, interpolation / 2.0 - 0.25),
            Vec2::new(width / 2.0, interpolation / 2.0 + 0.25),
            calculate_flip(head - neck),
        );
        push_circle(
            &mut verticies,
            head,
            (head - neck).as_vec2() * interpolation,
            head_size / 2.0,
        );

        // Interpolate tail
        push_quad(
            &mut verticies,
            tail,
            Vec2::new(0.0, interpolation / 2.0),
            Vec2::new(width / 2.0, -interpolation / 2.0),
            calculate_flip(snake.tail_dir),
        );
        push_circle(
            &mut verticies,
            tail,
            snake.tail_dir.as_vec2() * interpolation,
            width / 2.0,
        );
    }

    let mut last = head;
    for i in start..end {
        let pos = snake.body[i];

        push_circle(&mut verticies, pos, Vec2::new(0.0, 0.0), width / 2.0);

        if i > 0 {
            let flip1 = calculate_flip(last - pos);
            push_quad(
                &mut verticies,
                pos,
                Vec2::new(0.0, 0.25),
                Vec2::new(width / 2.0, 0.25),
                flip1,
            );
        }

        if i < len - 1 {
            let next = snake.body[i + 1];
            let flip2 = calculate_flip(next - pos);
            push_quad(
                &mut verticies,
                pos,
                Vec2::new(0.0, 0.25),
                Vec2::new(width / 2.0, 0.25),
                flip2,
            );
        }

        last = pos;
    }

    let mut positions = Vec::<[f32; 3]>::new();
    let mut normals = Vec::<[f32; 3]>::new();
    let mut uvs = Vec::<[f32; 2]>::new();
    for position in &verticies {
        positions.push(*position);
        normals.push([0.0, 0.0, 1.0]);
        uvs.push([0.0, 0.0]);
    }

    let mut snake_mesh = Mesh::new(PrimitiveTopology::TriangleList);
    snake_mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    snake_mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    snake_mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);

    snake_mesh
}