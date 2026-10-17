package shapes

type Point struct {
	X, Y float64
}

type Shaper interface {
	Area() float64
}

type Distance float64

func Origin() Point {
	return Point{}
}

func (p Point) Area() float64 {
	return 0
}

func (p *Point) Move(dx float64) {
	p.X += dx
}
