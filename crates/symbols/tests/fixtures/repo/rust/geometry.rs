mod shapes {
    pub fn unit() -> f64 {
        1.0
    }
}

pub struct Point {
    x: f64,
    y: f64,
}

pub enum Shape {
    Circle(f64),
    Square(f64),
}

pub trait Area {
    fn area(&self) -> f64;
}

pub type Distance = f64;

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }

    pub fn origin() -> Self {
        Point::new(0.0, 0.0)
    }
}

fn distance(a: &Point, b: &Point) -> Distance {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}
