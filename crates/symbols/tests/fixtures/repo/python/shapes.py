import math


class Circle:
    def __init__(self, radius):
        self.radius = radius

    def area(self):
        return math.pi * self.radius ** 2


class Square:
    def area(self):
        return 1


def total_area(shapes):
    return sum(shape.area() for shape in shapes)
